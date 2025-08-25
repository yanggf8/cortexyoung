/**
 * MCP Health Check Tools
 * Provides health monitoring and diagnostics accessible via MCP tools
 */

import { getMultiInstanceLogger, MultiInstanceLogger } from './multi-instance-logger';
import * as os from 'os';
import * as fs from 'fs';
import * as path from 'path';
import { execSync } from 'child_process';

export interface HealthCheckResult {
  overall: 'healthy' | 'degraded' | 'unhealthy';
  timestamp: string;
  instanceId: string;
  checks: {
    processHealth: boolean;
    resourceUsage: {
      memory: { used: number; total: number; percentage: number };
      cpu: number;
    };
    activeSessions: number;
    sessionConflicts: number;
    lastErrors: string[];
  };
  activeSessions: Array<{
    sessionId: string;
    pid: number;
    claudeInstance: string;
    status: string;
    uptime: string;
    healthStatus: 'good' | 'warning' | 'error';
  }>;
  recommendations: string[];
}

export class MCPHealthChecker {
  private logger: MultiInstanceLogger;

  constructor() {
    this.logger = getMultiInstanceLogger();
  }

  async performHealthCheck(): Promise<HealthCheckResult> {
    const timestamp = new Date().toISOString();
    const instanceId = (this.logger as any).instanceId;

    // Get all active sessions
    const sessions = MultiInstanceLogger.getActiveSessions();
    const activeSessions = Object.values(sessions);

    // Check for session conflicts
    const sessionConflicts = this.detectSessionConflicts(activeSessions);

    // Get resource usage
    const resourceUsage = await this.getResourceUsage();

    // Check process health
    const processHealth = this.checkProcessHealth();

    // Get recent errors
    const lastErrors = await this.getRecentErrors();

    // Analyze session health
    const sessionDetails = activeSessions.map(session => {
      const uptime = Math.floor((Date.now() - session.startTime) / 1000);
      const uptimeStr = this.formatUptime(uptime);
      
      let healthStatus: 'good' | 'warning' | 'error' = 'good';
      if (session.status === 'error') {
        healthStatus = 'error';
      } else if (Date.now() - session.lastActivity > 60000) { // No activity for 1 minute
        healthStatus = 'warning';
      } else if (session.healthChecks.length > 0) {
        const recentHealthChecks = session.healthChecks.slice(-3);
        const failureRate = recentHealthChecks.filter(hc => !hc.success).length / recentHealthChecks.length;
        if (failureRate > 0.5) {
          healthStatus = 'warning';
        }
      }

      return {
        sessionId: session.sessionId,
        pid: session.pid,
        claudeInstance: session.claudeInstance,
        status: session.status,
        uptime: uptimeStr,
        healthStatus
      };
    });

    // Determine overall health
    let overall: 'healthy' | 'degraded' | 'unhealthy' = 'healthy';
    if (sessionConflicts > 0 || lastErrors.length > 3) {
      overall = 'unhealthy';
    } else if (resourceUsage.memory.percentage > 80 || sessionDetails.some(s => s.healthStatus === 'warning')) {
      overall = 'degraded';
    }

    // Generate recommendations
    const recommendations = this.generateRecommendations(sessionConflicts, resourceUsage, sessionDetails);

    const result: HealthCheckResult = {
      overall,
      timestamp,
      instanceId,
      checks: {
        processHealth,
        resourceUsage,
        activeSessions: activeSessions.length,
        sessionConflicts,
        lastErrors
      },
      activeSessions: sessionDetails,
      recommendations
    };

    // Log health check
    this.logger.logHealthCheck(overall === 'healthy', overall !== 'healthy' ? `Health status: ${overall}` : undefined);

    return result;
  }

  private detectSessionConflicts(sessions: any[]): number {
    // Look for sessions with same project path but different PIDs
    const projectPaths = new Map<string, number>();
    
    for (const session of sessions) {
      const count = projectPaths.get(session.projectPath) || 0;
      projectPaths.set(session.projectPath, count + 1);
    }

    return Array.from(projectPaths.values()).filter(count => count > 1).length;
  }

  private async getResourceUsage() {
    const totalMemory = os.totalmem();
    const freeMemory = os.freemem();
    const usedMemory = totalMemory - freeMemory;

    // Get CPU usage (approximate)
    let cpuUsage = 0;
    try {
      const loadAvg = os.loadavg()[0]; // 1-minute load average
      const cpuCount = os.cpus().length;
      cpuUsage = Math.min((loadAvg / cpuCount) * 100, 100);
    } catch (error) {
      // Ignore CPU usage errors
    }

    return {
      memory: {
        used: usedMemory,
        total: totalMemory,
        percentage: Math.round((usedMemory / totalMemory) * 100)
      },
      cpu: Math.round(cpuUsage)
    };
  }

  private checkProcessHealth(): boolean {
    try {
      // Check if our process is still responsive
      return process.pid > 0 && typeof process.version === 'string';
    } catch (error) {
      return false;
    }
  }

  private async getRecentErrors(): Promise<string[]> {
    const errors: string[] = [];
    const instanceId = (this.logger as any).instanceId;
    const logFile = path.join(os.homedir(), '.cortex', 'multi-instance-logs', `${instanceId}.log`);

    if (!fs.existsSync(logFile)) {
      return errors;
    }

    try {
      const logContent = fs.readFileSync(logFile, 'utf8');
      const lines = logContent.trim().split('\n').slice(-50); // Last 50 lines

      for (const line of lines) {
        try {
          const entry = JSON.parse(line);
          if (entry.type === 'ERROR' || entry.message.toLowerCase().includes('error')) {
            errors.push(`${entry.timestamp}: ${entry.message}`);
          }
        } catch (parseError) {
          // Ignore parse errors
        }
      }
    } catch (error) {
      // Ignore read errors
    }

    return errors.slice(-5); // Return last 5 errors
  }

  private formatUptime(seconds: number): string {
    const hours = Math.floor(seconds / 3600);
    const minutes = Math.floor((seconds % 3600) / 60);
    const secs = seconds % 60;

    if (hours > 0) {
      return `${hours}h ${minutes}m ${secs}s`;
    } else if (minutes > 0) {
      return `${minutes}m ${secs}s`;
    } else {
      return `${secs}s`;
    }
  }

  private generateRecommendations(conflicts: number, resources: any, sessions: any[]): string[] {
    const recommendations: string[] = [];

    if (conflicts > 0) {
      recommendations.push('⚠️ Multiple sessions detected for same project - consider using single Claude Code instance');
    }

    if (resources.memory.percentage > 80) {
      recommendations.push('🔋 High memory usage detected - consider restarting Claude Code instances');
    }

    if (sessions.length > 5) {
      recommendations.push('📊 Many active sessions detected - consider closing unused Claude Code instances');
    }

    const erroringSessions = sessions.filter(s => s.healthStatus === 'error');
    if (erroringSessions.length > 0) {
      recommendations.push(`❌ ${erroringSessions.length} session(s) in error state - check individual session logs`);
    }

    const staleSessions = sessions.filter(s => s.healthStatus === 'warning');
    if (staleSessions.length > 0) {
      recommendations.push(`⏰ ${staleSessions.length} session(s) inactive - may need reconnection`);
    }

    if (recommendations.length === 0) {
      recommendations.push('✅ All systems operating normally');
    }

    return recommendations;
  }

  // MCP Tool Handler
  static async handleHealthCheckTool(args: any) {
    const checker = new MCPHealthChecker();
    const result = await checker.performHealthCheck();

    // Format for Claude Code display
    const summary = `## Cortex MCP Health Status: ${result.overall.toUpperCase()}

### System Overview
- **Instance**: ${result.instanceId}
- **Timestamp**: ${result.timestamp}
- **Active Sessions**: ${result.checks.activeSessions}
- **Session Conflicts**: ${result.checks.sessionConflicts}
- **Memory Usage**: ${result.checks.resourceUsage.memory.percentage}% (${Math.round(result.checks.resourceUsage.memory.used / 1024 / 1024 / 1024 * 10) / 10}GB)
- **CPU Usage**: ${result.checks.resourceUsage.cpu}%

### Active Sessions
${result.activeSessions.map(session => 
  `- **${session.claudeInstance}** (PID: ${session.pid}) - ${session.status} - ${session.uptime} - ${session.healthStatus === 'good' ? '✅' : session.healthStatus === 'warning' ? '⚠️' : '❌'}`
).join('\n')}

### Recent Errors
${result.checks.lastErrors.length > 0 ? result.checks.lastErrors.map(error => `- ${error}`).join('\n') : '- No recent errors'}

### Recommendations
${result.recommendations.map(rec => `- ${rec}`).join('\n')}

### Debug Information
- **Process Health**: ${result.checks.processHealth ? '✅ OK' : '❌ Issues detected'}
- **Log Location**: ~/.cortex/multi-instance-logs/
- **Session File**: ~/.cortex/multi-instance-logs/active-sessions.json
`;

    return {
      content: [
        {
          type: 'text',
          text: summary
        }
      ]
    };
  }
}