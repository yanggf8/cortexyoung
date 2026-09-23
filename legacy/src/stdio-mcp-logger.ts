import * as fs from 'fs';
import * as path from 'path';
import * as os from 'os';

export class StdioMcpLogger {
  private logDir: string;
  private sessionId: string;
  private claudeSession: string;
  private sessionLogFile: string;
  
  // Resource protection
  private maxLogSizeBytes = 100 * 1024 * 1024; // 100MB per log file
  private logWriteCount = 0;
  private logSizeCheckInterval = 100; // Check every 100 log writes

  constructor() {
    // Create claudecat log directory to avoid conflicts with other MCP servers
    const claudeDir = path.join(os.homedir(), '.claude');
    const logsDir = path.join(claudeDir, 'logs');
    if (!fs.existsSync(logsDir)) {
      fs.mkdirSync(logsDir, { recursive: true });
    }
    
    // Each session gets its own isolated log directory
    this.logDir = path.join(logsDir, 'claudecat');
    this.sessionId = this.generateSessionId();
    this.claudeSession = this.detectClaudeSession();
    
    // Session-specific log file (no shared logs)
    this.sessionLogFile = path.join(this.logDir, `session-${this.sessionId}.log`);
    
    // Create claudecat-specific logs directory
    if (!fs.existsSync(this.logDir)) {
      fs.mkdirSync(this.logDir, { recursive: true });
    }
    
    // Start logging (simplified)
    this.logToFile(`SESSION_START: ${this.claudeSession} (PID: ${process.pid}, Session: ${this.sessionId})`);
    
    // Setup cleanup handlers
    process.on('exit', () => this.cleanup());
    process.on('SIGINT', () => { this.cleanup(); process.exit(0); });
    process.on('SIGTERM', () => { this.cleanup(); process.exit(0); });
  }
  
  getSessionId(): string {
    return this.sessionId;
  }

  getClaudeSession(): string {
    return this.claudeSession;
  }

  private generateSessionId(): string {
    const timestamp = Date.now().toString(36);
    const random = Math.random().toString(36).substring(2, 8);
    return `claudecat-${timestamp}-${random}`;
  }
  
  private detectClaudeSession(): string {
    try {
      // Multiple detection methods for Claude Code instances
      const methods = [
        () => process.env.CLAUDE_SESSION_ID,
        () => process.env.CLAUDE_DESKTOP_SESSION,
        () => `claude-${process.ppid}`,
        () => `claude-time-${Date.now()}`
      ];
      
      for (const method of methods) {
        const result = method();
        if (result) {
          return result;
        }
      }
      
      return `unknown-claude-${Date.now()}`;
    } catch (error) {
      return `error-claude-${Date.now()}`;
    }
  }
  
  // Removed: No more shared session registration
  
  logToFile(message: string): void {
    try {
      // Check log file size periodically to prevent runaway growth
      this.logWriteCount++;
      if (this.logWriteCount % this.logSizeCheckInterval === 0) {
        this.checkAndRotateLog();
      }
      
      const timestamp = new Date().toISOString();
      const logLine = `[${timestamp}] [${this.claudeSession}] [${this.sessionId}] ${message}\n`;
      
      // Write only to session-specific file (no console.error to avoid EPIPE loops)
      fs.appendFileSync(this.sessionLogFile, logLine);
      
    } catch (error) {
      // Silent failure to prevent infinite loops - no console.error at all
      try {
        const errorLine = `[${new Date().toISOString()}] [${this.claudeSession}] [${this.sessionId}] LOG_ERROR: ${error instanceof Error ? error.message : String(error)}\n`;
        fs.appendFileSync(this.sessionLogFile, errorLine);
      } catch {
        // Complete silent failure if file system has issues
      }
    }
  }
  
  private checkAndRotateLog(): void {
    try {
      const stats = fs.statSync(this.sessionLogFile);
      if (stats.size > this.maxLogSizeBytes) {
        const rotatedFile = `${this.sessionLogFile}.${Date.now()}.old`;
        fs.renameSync(this.sessionLogFile, rotatedFile);
        this.logToFile('LOG_ROTATED: Log file rotated due to size limit');
      }
    } catch {
      // Ignore rotation errors
    }
  }
  
  updateSessionStatus(status: string): void {
    // Simplified: just log status changes to own file
    this.logToFile(`SESSION_STATUS_UPDATED: ${status}`);
  }

  getActiveSessionsInfo(): any {
    try {
      // Discover sessions by scanning log directory
      const logFiles = fs.readdirSync(this.logDir)
        .filter(file => file.startsWith('session-') && file.endsWith('.log'))
        .map(file => {
          const sessionId = file.replace('session-', '').replace('.log', '');
          const filePath = path.join(this.logDir, file);
          try {
            const stats = fs.statSync(filePath);
            return {
              sessionId,
              logFile: file,
              size: stats.size,
              modified: stats.mtime,
              uptime: Math.round((Date.now() - stats.birthtime.getTime()) / 1000)
            };
          } catch {
            return null;
          }
        })
        .filter(Boolean);
      
      return {
        totalSessions: logFiles.length,
        currentSession: this.sessionId,
        claudeSession: this.claudeSession,
        sessionDetails: logFiles
      };
    } catch (error) {
      return { error: error instanceof Error ? error.message : String(error) };
    }
  }
  
  cleanup(): void {
    this.logToFile('SESSION_END: MCP server shutting down');
    // No shared resources to clean up - session isolation complete
  }
}