/**
 * Simple timestamped logging utilities
 * Provides consistent timestamp formatting across console logging
 */

/**
 * Log with ISO timestamp prefix, preserving emoji and formatting
 */
export const timestampedLog = (message: string, ...args: any[]): void => {
  const timestamp = new Date().toISOString();
  console.log(`[${timestamp}] ${message}`, ...args);
};

/**
 * Convenience methods for different log levels
 */
export const log = timestampedLog;

export const warn = (message: string, ...args: any[]): void => {
  const timestamp = new Date().toISOString();
  console.warn(`[${timestamp}] ${message}`, ...args);
};

export const error = (message: string, ...args: any[]): void => {
  const timestamp = new Date().toISOString();
  console.error(`[${timestamp}] ${message}`, ...args);
};

/**
 * Logger object with common logging methods
 */
export const logger = {
  log: timestampedLog,
  warn,
  error,
  info: timestampedLog,  // Alias for log
  debug: timestampedLog  // Alias for log
};