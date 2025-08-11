/**
 * Simple timestamped logging utilities
 * Provides consistent timestamp formatting across console logging
 */

/**
 * Log with ISO timestamp prefix, preserving emoji and formatting
 */
export const timestampedLog = (message: string): void => {
  const timestamp = new Date().toISOString();
  console.log(`[${timestamp}] ${message}`);
};

/**
 * Convenience methods for different log levels
 */
export const log = timestampedLog;

export const warn = (message: string): void => {
  const timestamp = new Date().toISOString();
  console.warn(`[${timestamp}] ${message}`);
};

export const error = (message: string): void => {
  const timestamp = new Date().toISOString();
  console.error(`[${timestamp}] ${message}`);
};