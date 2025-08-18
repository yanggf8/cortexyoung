/**
 * Configurable Enhanced Console Logger
 * Built on top of existing console-logger with configuration system integration
 */

import { getLoggerConfig, LoggerConfig } from './logger-config';
import { ConsoleLogger } from './console-logger';
import { formatJson, formatTable, formatBox, formatProgress } from './advanced-formatters';

// Create a configurable logger instance that adapts to the current configuration
class ConfigurableLogger {
  private baseLogger: ConsoleLogger;
  
  constructor() {
    this.baseLogger = new ConsoleLogger();
  }
  
  // Update logger behavior based on current configuration
  private updateLoggerFromConfig(): void {
    const config = getLoggerConfig();
    
    // Update base logger configuration
    this.baseLogger.setColorEnabled(config.colors);
    this.baseLogger.setEmojiEnabled(config.emojis);
    
    // Handle log level filtering at the method level
  }
  
  // Check if message should be logged based on configuration
  private shouldLog(level: 'debug' | 'info' | 'warn' | 'error'): boolean {
    const config = getLoggerConfig();
    
    if (!config.enabled) return false;
    
    const levels = { debug: 0, info: 1, warn: 2, error: 3, silent: 4 };
    const currentLevel = levels[config.logLevel];
    const messageLevel = levels[level];
    
    return messageLevel >= currentLevel;
  }
  
  // Core logging methods with configuration support
  debug(message: string, metadata?: Record<string, any>): void {
    if (!this.shouldLog('debug')) return;
    this.updateLoggerFromConfig();
    
    const config = getLoggerConfig();
    if (config.timestamps) {
      this.baseLogger.debug(message, metadata);
    } else {
      // Use base logger without timestamp
      this.baseLogger.info(message, metadata);
    }
  }
  
  info(message: string, metadata?: Record<string, any>): void {
    if (!this.shouldLog('info')) return;
    this.updateLoggerFromConfig();
    this.baseLogger.info(message, metadata);
  }
  
  warn(message: string, metadata?: Record<string, any>): void {
    if (!this.shouldLog('warn')) return;
    this.updateLoggerFromConfig();
    this.baseLogger.warn(message, metadata);
  }
  
  error(message: string, metadata?: Record<string, any>): void {
    if (!this.shouldLog('error')) return;
    this.updateLoggerFromConfig();
    this.baseLogger.error(message, metadata);
  }
  
  success(message: string, metadata?: Record<string, any>): void {
    if (!this.shouldLog('info')) return;
    this.updateLoggerFromConfig();
    this.baseLogger.success(message, metadata);
  }
  
  ready(message: string, metadata?: Record<string, any>): void {
    if (!this.shouldLog('info')) return;
    this.updateLoggerFromConfig();
    this.baseLogger.ready(message, metadata);
  }
  
  // Stage management with configuration support
  stage = {
    start: (stageNumber: number, totalStages: number, name: string): void => {
      if (!this.shouldLog('info')) return;
      const config = getLoggerConfig();
      
      this.updateLoggerFromConfig();
      
      if (config.stageDelimiters) {
        this.baseLogger.stage.start(stageNumber, totalStages, name);
      } else {
        // Simple stage start without delimiters
        this.baseLogger.info(`STAGE ${stageNumber}/${totalStages}: ${name}`);
      }
    },
    
    complete: (result?: string): void => {
      if (!this.shouldLog('info')) return;
      const config = getLoggerConfig();
      
      this.updateLoggerFromConfig();
      
      if (config.stageDelimiters) {
        this.baseLogger.stage.complete(result);
      } else {
        this.baseLogger.success(`Stage completed${result ? ': ' + result : ''}`);
      }
    },
    
    fail: (error: string): void => {
      if (!this.shouldLog('error')) return;
      this.updateLoggerFromConfig();
      this.baseLogger.stage.fail(error);
    }
  };
  
  // Step management with configuration support  
  step = {
    start: (stepId: string, name: string, description?: string): void => {
      if (!this.shouldLog('info')) return;
      const config = getLoggerConfig();
      
      this.updateLoggerFromConfig();
      
      if (config.stepDelimiters) {
        this.baseLogger.step.start(stepId, name, description);
      } else {
        // Simple step start without delimiters
        this.baseLogger.info(`STEP ${stepId}: ${name}`);
        if (description && config.metadata) {
          this.baseLogger.info(`  ${description}`);
        }
      }
    },
    
    complete: (result?: string): void => {
      if (!this.shouldLog('info')) return;
      this.updateLoggerFromConfig();
      this.baseLogger.step.complete(result);
    },
    
    fail: (error: string, hint?: string): void => {
      if (!this.shouldLog('error')) return;
      this.updateLoggerFromConfig();
      this.baseLogger.step.fail(error, hint);
    },
    
    update: (message: string, metadata?: Record<string, any>): void => {
      if (!this.shouldLog('info')) return;
      this.updateLoggerFromConfig();
      this.baseLogger.step.update(message, metadata);
    }
  };
  
  // Progress indicators with configuration support
  progress(current: number, total: number, message?: string): void {
    const config = getLoggerConfig();
    if (!this.shouldLog('info') || !config.progressIndicators) return;
    
    this.updateLoggerFromConfig();
    
    const progressBar = formatProgress(current, total, {
      width: 30,
      showPercentage: true,
      showNumbers: false
    });
    
    const progressMessage = message ? `${message} ${progressBar}` : progressBar;
    this.baseLogger.info(progressMessage);
  }
  
  // Structured data logging with advanced formatters
  data = {
    json: (data: any, title?: string): void => {
      if (!this.shouldLog('info')) return;
      const config = getLoggerConfig();
      
      if (title) {
        this.info(title);
      }
      
      const lines = formatJson(data, { 
        indent: config.indentSize || 2, 
        colorize: config.colors 
      });
      
      lines.forEach(line => console.log(line));
    },
    
    table: (data: any[], columns?: any[], title?: string): void => {
      if (!this.shouldLog('info')) return;
      
      if (title) {
        this.info(title);
      }
      
      const tableColumns = columns || (data.length > 0 ? 
        Object.keys(data[0]).map(key => ({ key, title: key })) : []);
      
      const lines = formatTable({
        columns: tableColumns,
        data,
        border: true,
        header: true
      });
      
      lines.forEach(line => console.log(line));
    },
    
    box: (content: string[], title?: string): void => {
      if (!this.shouldLog('info')) return;
      
      const lines = formatBox(content, {
        title,
        borderColor: 'border',
        titleColor: 'header'
      });
      
      lines.forEach(line => console.log(line));
    }
  };
  
  // Configuration management
  updateConfig(updates: Partial<LoggerConfig>): void {
    const { updateLoggerConfig } = require('./logger-config');
    updateLoggerConfig(updates);
  }
}

// Create and export singleton instance
const configurableLogger = new ConfigurableLogger();

// Export individual functions for easy access
export const debug = configurableLogger.debug.bind(configurableLogger);
export const info = configurableLogger.info.bind(configurableLogger);
export const warn = configurableLogger.warn.bind(configurableLogger);
export const error = configurableLogger.error.bind(configurableLogger);
export const success = configurableLogger.success.bind(configurableLogger);
export const ready = configurableLogger.ready.bind(configurableLogger);

export const stage = configurableLogger.stage;
export const step = configurableLogger.step;
export const progress = configurableLogger.progress.bind(configurableLogger);
export const data = configurableLogger.data;

// Main logger object for compatibility
export const logger = {
  debug,
  info, 
  warn,
  error,
  success,
  ready,
  stage,
  step,
  progress,
  data,
  updateConfig: configurableLogger.updateConfig.bind(configurableLogger),
  // Legacy compatibility
  log: info,
};

// Export the class for advanced use
export { ConfigurableLogger };

export default logger;