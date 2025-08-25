/**
 * Multi-Instance Logging System
 * Tracks all Cortex MCP server instances across multiple Claude Code sessions
 */

import * as fs from 'fs';
import * as path from 'path';
import * as os from 'os';
import { execSync } from 'child_process';

interface InstanceSession {
  sessionId: string;
  pid: number;
  parentPid: number;
  startTime: number;
  claudeInstance: string;
  projectPath: string;
  status: 'starting' | 'ready' | 'error' | 'disconnected';
  lastActivity: number;
  healthChecks: {
    timestamp: number;
    success: boolean;
    error?: string;
  }[];
  environment: {
    multiInstance: boolean;
    skipCleanup: boolean;
    skipHealthCheck: boolean;
  };
}

export class MultiInstanceLogger {
  private logDir: string;
  private sessionFile: string;
  private instanceId: string;
  private session: InstanceSession;

  constructor(projectPath: string = process.cwd()) {
    // Create logs directory
    this.logDir = path.join(os.homedir(), '.cortex', 'multi-instance-logs');
    if (!fs.existsSync(this.logDir)) {
      fs.mkdirSync(this.logDir, { recursive: true });
    }

    this.sessionFile = path.join(this.logDir, 'active-sessions.json');
    this.instanceId = this.generateInstanceId();
    
    // Initialize session
    this.session = {
      sessionId: this.instanceId,
      pid: process.pid,
      parentPid: process.ppid || 0,
      startTime: Date.now(),
      claudeInstance: this.detectClaudeInstance(),
      projectPath: projectPath,
      status: 'starting',
      lastActivity: Date.now(),
      healthChecks: [],
      environment: {
        multiInstance: !!process.env.MCP_MULTI_INSTANCE,
        skipCleanup: !!process.env.CORTEX_SKIP_CLEANUP,
        skipHealthCheck: !!process.env.CORTEX_SKIP_HEALTH_CHECK
      }
    };

    this.registerSession();
    this.logActivity('STARTUP', `Cortex MCP instance started (PID: ${process.pid})`);
  }

  private generateInstanceId(): string {
    const timestamp = Date.now().toString(36);
    const random = Math.random().toString(36).substring(2, 8);
    return `cortex-${timestamp}-${random}`;
  }

  private detectClaudeInstance(): string {
    try {
      // Try to detect Claude Code instance via parent process
      const ppid = process.ppid;
      if (ppid) {
        const parentCmd = execSync(`ps -p ${ppid} -o comm=`, { encoding: 'utf8' }).trim();
        if (parentCmd.includes('claude') || parentCmd.includes('node')) {
          return `claude-${ppid}`;
        }
      }
    } catch (error) {
      // Ignore detection errors
    }
    
    return `unknown-${process.ppid || 'no-parent'}`;
  }

  private registerSession(): void {
    try {
      let sessions: Record<string, InstanceSession> = {};
      
      if (fs.existsSync(this.sessionFile)) {
        const data = fs.readFileSync(this.sessionFile, 'utf8');
        sessions = JSON.parse(data);
      }

      sessions[this.instanceId] = this.session;
      fs.writeFileSync(this.sessionFile, JSON.stringify(sessions, null, 2));
    } catch (error) {
      console.error('[MultiInstanceLogger] Failed to register session:', error);
    }
  }

  logActivity(type: string, message: string, metadata?: any): void {
    const timestamp = new Date().toISOString();
    const logEntry = {
      timestamp,
      sessionId: this.instanceId,
      pid: process.pid,
      claudeInstance: this.session.claudeInstance,
      type,
      message,
      metadata: metadata || {}
    };

    // Update session activity
    this.session.lastActivity = Date.now();
    this.updateSession();

    // Log to individual session file
    const sessionLogFile = path.join(this.logDir, `${this.instanceId}.log`);
    const logLine = JSON.stringify(logEntry) + '\n';
    
    try {
      fs.appendFileSync(sessionLogFile, logLine);
    } catch (error) {
      console.error('[MultiInstanceLogger] Failed to write log:', error);
    }

    // Also log to stderr for immediate visibility
    console.error(`[${timestamp}] [${this.instanceId}] [${type}] ${message}`);
    if (metadata && Object.keys(metadata).length > 0) {
      console.error(`[${timestamp}] [${this.instanceId}] [METADATA]`, JSON.stringify(metadata));
    }
  }

  logHealthCheck(success: boolean, error?: string): void {
    const healthCheck = {
      timestamp: Date.now(),
      success,
      error
    };

    this.session.healthChecks.push(healthCheck);
    
    // Keep only last 10 health checks
    if (this.session.healthChecks.length > 10) {
      this.session.healthChecks = this.session.healthChecks.slice(-10);
    }

    this.updateSession();
    this.logActivity('HEALTH_CHECK', success ? 'Health check passed' : 'Health check failed', { error });
  }

  updateStatus(status: InstanceSession['status']): void {
    this.session.status = status;
    this.session.lastActivity = Date.now();
    this.updateSession();
    this.logActivity('STATUS_CHANGE', `Status changed to: ${status}`);
  }

  private updateSession(): void {
    try {
      let sessions: Record<string, InstanceSession> = {};
      
      if (fs.existsSync(this.sessionFile)) {
        const data = fs.readFileSync(this.sessionFile, 'utf8');
        sessions = JSON.parse(data);
      }

      sessions[this.instanceId] = this.session;
      fs.writeFileSync(this.sessionFile, JSON.stringify(sessions, null, 2));
    } catch (error) {
      console.error('[MultiInstanceLogger] Failed to update session:', error);
    }
  }

  cleanup(): void {
    this.logActivity('SHUTDOWN', 'Cortex MCP instance shutting down');
    
    try {
      if (fs.existsSync(this.sessionFile)) {
        const data = fs.readFileSync(this.sessionFile, 'utf8');
        const sessions = JSON.parse(data);
        delete sessions[this.instanceId];
        fs.writeFileSync(this.sessionFile, JSON.stringify(sessions, null, 2));
      }
    } catch (error) {
      console.error('[MultiInstanceLogger] Failed to cleanup session:', error);
    }
  }

  static getActiveSessions(): Record<string, InstanceSession> {
    const sessionFile = path.join(os.homedir(), '.cortex', 'multi-instance-logs', 'active-sessions.json');
    
    if (!fs.existsSync(sessionFile)) {
      return {};
    }

    try {
      const data = fs.readFileSync(sessionFile, 'utf8');
      const sessions = JSON.parse(data);
      
      // Filter out stale sessions (older than 10 minutes with no activity)
      const now = Date.now();
      const staleCutoff = 10 * 60 * 1000; // 10 minutes
      
      const activeSessions: Record<string, InstanceSession> = {};
      for (const [sessionId, session] of Object.entries(sessions)) {
        const typedSession = session as InstanceSession;
        if (now - typedSession.lastActivity < staleCutoff) {
          activeSessions[sessionId] = typedSession;
        }
      }
      
      return activeSessions;
    } catch (error) {
      console.error('[MultiInstanceLogger] Failed to read sessions:', error);
      return {};
    }
  }

  static logGlobalEvent(type: string, message: string, metadata?: any): void {
    const timestamp = new Date().toISOString();
    const logDir = path.join(os.homedir(), '.cortex', 'multi-instance-logs');
    
    if (!fs.existsSync(logDir)) {
      fs.mkdirSync(logDir, { recursive: true });
    }

    const globalLogFile = path.join(logDir, 'global.log');
    const logEntry = {
      timestamp,
      type,
      message,
      metadata: metadata || {}
    };

    const logLine = JSON.stringify(logEntry) + '\n';
    
    try {
      fs.appendFileSync(globalLogFile, logLine);
    } catch (error) {
      console.error('[MultiInstanceLogger] Failed to write global log:', error);
    }

    console.error(`[${timestamp}] [GLOBAL] [${type}] ${message}`);
  }
}

// Singleton instance
let logger: MultiInstanceLogger | null = null;

export function getMultiInstanceLogger(projectPath?: string): MultiInstanceLogger {
  if (!logger) {
    logger = new MultiInstanceLogger(projectPath);
    
    // Setup cleanup on exit
    process.on('exit', () => {
      if (logger) {
        logger.cleanup();
      }
    });

    process.on('SIGINT', () => {
      if (logger) {
        logger.cleanup();
      }
      process.exit(0);
    });

    process.on('SIGTERM', () => {
      if (logger) {
        logger.cleanup();
      }
      process.exit(0);
    });
  }
  
  return logger;
}