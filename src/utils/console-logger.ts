/**
 * Enhanced console logger with colors, emojis, and structured formatting
 * Provides scannable startup logs with consistent status indicators
 */

// Track process start time for 'sofar' calculations
const PROCESS_START_TIME = process.hrtime.bigint();

// Environment detection
const isColorSupported = (): boolean => {
  if (process.env.NO_COLOR) return false;
  if (process.env.FORCE_COLOR) return true;
  if (!process.stdout.isTTY) return false;
  return true;
};

const isBlinkSupported = (): boolean => {
  if (process.env.DISABLE_BLINK) return false;
  return isColorSupported();
};

// ANSI color codes
const colors = {
  reset: '\x1b[0m',
  bright: '\x1b[1m',
  dim: '\x1b[2m',
  
  // Foreground colors
  red: '\x1b[91m',      // bright red
  green: '\x1b[92m',    // bright green
  yellow: '\x1b[93m',   // bright yellow
  blue: '\x1b[94m',     // bright blue
  magenta: '\x1b[95m',  // bright magenta
  cyan: '\x1b[96m',     // bright cyan
  white: '\x1b[97m',    // bright white
  gray: '\x1b[90m',     // dark gray
  
  // Special effects
  blink: '\x1b[5m',
  underline: '\x1b[4m',
} as const;

// Color helper function
const colorize = (text: string, color: keyof typeof colors): string => {
  if (!isColorSupported()) return text;
  return `${colors[color]}${text}${colors.reset}`;
};

// Blink helper for critical failures
const blinkText = (text: string): string => {
  if (!isBlinkSupported()) return text;
  return `${colors.blink}${text}${colors.reset}`;
};

// Status tokens with emoji fallbacks
const statusTokens = {
  start: { emoji: '⚡', text: 'START', color: 'cyan' as const },
  ok: { emoji: '✅', text: 'OK', color: 'green' as const },
  warn: { emoji: '⚠️', text: 'WARN', color: 'yellow' as const },
  fail: { emoji: '❌', text: 'FAIL', color: 'red' as const },
  ready: { emoji: '🎉', text: 'READY', color: 'green' as const },
};

// Format status indicator
const formatStatus = (status: keyof typeof statusTokens): string => {
  const token = statusTokens[status];
  const statusText = `${token.emoji} ${token.text}`;
  
  if (status === 'fail') {
    return blinkText(colorize(statusText, token.color));
  }
  
  if (status === 'ready') {
    return colorize(`${colors.bright}${statusText}`, token.color);
  }
  
  return colorize(statusText, token.color);
};

// Timing utilities
export const formatDuration = (ms: number): string => {
  if (ms < 1000) return `${Math.round(ms)}ms`;
  const seconds = ms / 1000;
  if (seconds < 10) return `${seconds.toFixed(1)}s`;
  return `${seconds.toFixed(0)}s`;
};

export const calculateSofar = (): string => {
  const nowNs = process.hrtime.bigint();
  const elapsedNs = nowNs - PROCESS_START_TIME;
  const elapsedMs = Number(elapsedNs) / 1_000_000;
  return formatDuration(elapsedMs);
};

// Metadata formatting
interface LogMetadata {
  dur?: string;
  sofar?: string;
  [key: string]: any;
}

const formatMetadata = (metadata: LogMetadata): string => {
  if (!metadata || Object.keys(metadata).length === 0) return '';
  
  const items: string[] = [];
  
  // Always show timing first if present
  if (metadata.dur) {
    items.push(`${colorize('dur:', 'dim')} ${colorize(metadata.dur, 'yellow')}`);
  }
  if (metadata.sofar) {
    items.push(`${colorize('sofar:', 'dim')} ${colorize(metadata.sofar, 'yellow')}`);
  }
  
  // Then other metadata
  Object.entries(metadata).forEach(([key, value]) => {
    if (key === 'dur' || key === 'sofar') return; // Already handled
    
    const keyStr = colorize(`${key}:`, 'dim');
    const valueStr = colorize(String(value), 'cyan');
    items.push(`${keyStr} ${valueStr}`);
  });
  
  if (items.length === 0) return '';
  
  return '\n  • ' + items.join(' • ');
};

// Core logging interface
interface LogOptions {
  timestamp?: Date;
  metadata?: LogMetadata;
  reason?: string;
  hint?: string;
}

// Main logging functions
export const start = (message: string, options: LogOptions = {}): void => {
  const timestamp = options.timestamp || new Date();
  const timestampStr = colorize(`[${timestamp.toISOString()}]`, 'gray');
  const status = formatStatus('start');
  const messageStr = colorize(message, 'white');
  
  let output = `${timestampStr} ${status}  ${messageStr}`;
  
  if (options.metadata) {
    output += formatMetadata(options.metadata);
  }
  
  console.log(output);
};

export const ok = (message: string, options: LogOptions = {}): void => {
  const timestamp = options.timestamp || new Date();
  const timestampStr = colorize(`[${timestamp.toISOString()}]`, 'gray');
  const status = formatStatus('ok');
  const messageStr = colorize(message, 'white');
  
  let output = `${timestampStr} ${status}     ${messageStr}`;
  
  if (options.metadata) {
    output += formatMetadata(options.metadata);
  }
  
  console.log(output);
};

export const warn = (message: string, options: LogOptions = {}): void => {
  const timestamp = options.timestamp || new Date();
  const timestampStr = colorize(`[${timestamp.toISOString()}]`, 'gray');
  const status = formatStatus('warn');
  const messageStr = colorize(message, 'white');
  
  let output = `${timestampStr} ${status}   ${messageStr}`;
  
  if (options.metadata) {
    output += formatMetadata(options.metadata);
  }
  
  if (options.reason) {
    output += `\n  • ${colorize('reason:', 'dim')} ${colorize(options.reason, 'yellow')}`;
  }
  
  console.warn(output);
};

export const fail = (message: string, options: LogOptions = {}): void => {
  const timestamp = options.timestamp || new Date();
  const timestampStr = colorize(`[${timestamp.toISOString()}]`, 'gray');
  const status = formatStatus('fail');
  const messageStr = colorize(message, 'white');
  
  let output = `${timestampStr} ${status}   ${messageStr}`;
  
  if (options.metadata) {
    output += formatMetadata(options.metadata);
  }
  
  if (options.reason) {
    output += `\n  • ${colorize('reason:', 'dim')} ${colorize(options.reason, 'red')}`;
  }
  
  if (options.hint) {
    output += `\n  • ${colorize('hint:', 'dim')} ${colorize(options.hint, 'cyan')}`;
  }
  
  console.error(output);
};

export const ready = (message: string, options: LogOptions = {}): void => {
  const timestamp = options.timestamp || new Date();
  const timestampStr = colorize(`[${timestamp.toISOString()}]`, 'gray');
  const status = formatStatus('ready');
  const messageStr = colorize(`${colors.bright}${message}`, 'white');
  
  let output = `${timestampStr} ${status}  ${messageStr}`;
  
  if (options.metadata) {
    output += formatMetadata(options.metadata);
  }
  
  console.log(output);
};

// Logger object for compatibility
export const logger = {
  start,
  ok,
  warn,
  fail,
  ready,
  
  // Legacy compatibility
  log: (message: string) => ok(message),
  info: (message: string) => ok(message),
  error: (message: string) => fail(message),
  debug: (message: string) => ok(message),
};

// Feature flag support
export const isNewLoggingEnabled = (): boolean => {
  return process.env.ENABLE_NEW_LOGGING === 'true';
};

// Stage and Step tracking
interface StageContext {
  stageNumber: number;
  totalStages: number;
  title: string;
  startTime: number;
}

interface StepContext {
  stepId: string;
  title: string;
  startTime: number;
  stageContext?: StageContext;
}

// Internal tracking
let currentStage: StageContext | null = null;
let currentStep: StepContext | null = null;

// Auto-timing utilities
export const autoTime = {
  startTime: Date.now(),
  
  getSofar: (): string => {
    return calculateSofar();
  },
  
  getDuration: (startTime: number): string => {
    const duration = Date.now() - startTime;
    return formatDuration(duration);
  }
};

// Stage management API
export const stage = {
  start: (stageNumber: number, totalStages: number, title: string): void => {
    const startTime = Date.now();
    currentStage = { stageNumber, totalStages, title, startTime };
    
    if (isNewLoggingEnabled()) {
      console.log(''); // Blank line for separation
      console.log(colorize('==========================================', 'gray'));
      start(`STAGE ${stageNumber}/${totalStages}: ${title.toUpperCase()}`, {
        metadata: { sofar: autoTime.getSofar() }
      });
      console.log(colorize('==========================================', 'gray'));
    } else {
      const timestamp = new Date().toISOString();
      console.log(`[${timestamp}] [INFO] 🚀 STAGE ${stageNumber}/${totalStages}: ${title.toUpperCase()}`);
    }
  },
  
  complete: (result?: string): void => {
    if (!currentStage) return;
    
    const duration = autoTime.getDuration(currentStage.startTime);
    const sofar = autoTime.getSofar();
    
    if (isNewLoggingEnabled()) {
      console.log(colorize('==========================================', 'gray'));
      ok(`STAGE ${currentStage.stageNumber}/${currentStage.totalStages} COMPLETED: ${currentStage.title.toUpperCase()}`, {
        metadata: { dur: duration, sofar, result: result || 'Success' }
      });
      console.log(colorize('==========================================', 'gray'));
    } else {
      const timestamp = new Date().toISOString();
      console.log(`[${timestamp}] [INFO] ✅ STAGE ${currentStage.stageNumber}/${currentStage.totalStages} COMPLETED: ${currentStage.title.toUpperCase()}`);
    }
    
    currentStage = null;
  },
  
  fail: (error: string): void => {
    if (!currentStage) return;
    
    const duration = autoTime.getDuration(currentStage.startTime);
    const sofar = autoTime.getSofar();
    
    if (isNewLoggingEnabled()) {
      console.log(colorize('==========================================', 'gray'));
      fail(`STAGE ${currentStage.stageNumber}/${currentStage.totalStages} FAILED: ${currentStage.title.toUpperCase()}`, {
        metadata: { dur: duration, sofar },
        reason: error
      });
      console.log(colorize('==========================================', 'gray'));
    } else {
      const timestamp = new Date().toISOString();
      console.error(`[${timestamp}] [ERROR] ❌ STAGE ${currentStage.stageNumber}/${currentStage.totalStages} FAILED: ${currentStage.title.toUpperCase()}`);
    }
    
    currentStage = null;
  }
};

// Step management API
export const step = {
  start: (stepId: string, title: string, description?: string): void => {
    const startTime = Date.now();
    currentStep = { stepId, title, startTime, stageContext: currentStage || undefined };
    
    if (isNewLoggingEnabled()) {
      console.log(colorize('------------------------------------------', 'gray'));
      start(`STEP ${stepId}: ${title}`, {
        metadata: { sofar: autoTime.getSofar() }
      });
      if (description) {
        console.log(`   ${colorize('Details:', 'dim')} ${colorize(description, 'white')}`);
      }
      console.log(colorize('------------------------------------------', 'gray'));
    } else {
      const timestamp = new Date().toISOString();
      console.log(`[${timestamp}] [INFO] ⚡ STEP ${stepId}: ${title}`);
    }
  },
  
  complete: (result?: string): void => {
    if (!currentStep) return;
    
    const duration = autoTime.getDuration(currentStep.startTime);
    const sofar = autoTime.getSofar();
    
    if (isNewLoggingEnabled()) {
      console.log(colorize('------------------------------------------', 'gray'));
      ok(`STEP ${currentStep.stepId} COMPLETED: ${currentStep.title}`, {
        metadata: { dur: duration, sofar, result: result || 'Success' }
      });
      console.log(colorize('------------------------------------------', 'gray'));
    } else {
      const timestamp = new Date().toISOString();
      console.log(`[${timestamp}] [INFO] ✅ STEP ${currentStep.stepId} COMPLETED: ${currentStep.title}`);
    }
    
    currentStep = null;
  },
  
  fail: (error: string, hint?: string): void => {
    if (!currentStep) return;
    
    const duration = autoTime.getDuration(currentStep.startTime);
    const sofar = autoTime.getSofar();
    
    if (isNewLoggingEnabled()) {
      console.log(colorize('------------------------------------------', 'gray'));
      fail(`STEP ${currentStep.stepId} FAILED: ${currentStep.title}`, {
        metadata: { dur: duration, sofar },
        reason: error,
        hint
      });
      console.log(colorize('------------------------------------------', 'gray'));
    } else {
      const timestamp = new Date().toISOString();
      console.error(`[${timestamp}] [ERROR] ❌ STEP ${currentStep.stepId} FAILED: ${currentStep.title}`);
    }
    
    currentStep = null;
  },
  
  update: (message: string, metadata?: LogMetadata): void => {
    if (!currentStep) return;
    
    const sofar = autoTime.getSofar();
    const enhancedMetadata = { sofar, ...metadata };
    
    if (isNewLoggingEnabled()) {
      ok(message, { metadata: enhancedMetadata });
    } else {
      const timestamp = new Date().toISOString();
      console.log(`[${timestamp}] [INFO] ${message}`);
    }
  }
};

// Enhanced logger object with new API
export const log = {
  // Basic logging
  start,
  ok,
  warn,
  fail,
  ready,
  
  // Stage/Step API
  stage,
  step,
  
  // Auto-timing utilities
  autoTime,
  
  // Legacy compatibility
  info: (message: string) => ok(message),
  error: (message: string) => fail(message),
  debug: (message: string) => ok(message),
};

// Conditional logger that respects feature flag
export const conditionalLogger = {
  start: (message: string, options?: LogOptions) => {
    if (isNewLoggingEnabled()) {
      start(message, options);
    } else {
      // Fallback to old logging
      const timestamp = new Date().toISOString();
      console.log(`[${timestamp}] ${message}`);
    }
  },
  
  ok: (message: string, options?: LogOptions) => {
    if (isNewLoggingEnabled()) {
      ok(message, options);
    } else {
      const timestamp = new Date().toISOString();
      console.log(`[${timestamp}] ${message}`);
    }
  },
  
  warn: (message: string, options?: LogOptions) => {
    if (isNewLoggingEnabled()) {
      warn(message, options);
    } else {
      const timestamp = new Date().toISOString();
      console.warn(`[${timestamp}] ${message}`);
    }
  },
  
  fail: (message: string, options?: LogOptions) => {
    if (isNewLoggingEnabled()) {
      fail(message, options);
    } else {
      const timestamp = new Date().toISOString();
      console.error(`[${timestamp}] ${message}`);
    }
  },
  
  ready: (message: string, options?: LogOptions) => {
    if (isNewLoggingEnabled()) {
      ready(message, options);
    } else {
      const timestamp = new Date().toISOString();
      console.log(`[${timestamp}] ${message}`);
    }
  },
  
  // Enhanced API with feature flag support
  stage: {
    start: (stageNumber: number, totalStages: number, title: string) => {
      if (isNewLoggingEnabled()) {
        stage.start(stageNumber, totalStages, title);
      } else {
        const timestamp = new Date().toISOString();
        console.log(`[${timestamp}] [INFO] 🚀 STAGE ${stageNumber}/${totalStages}: ${title.toUpperCase()}`);
      }
    },
    
    complete: (result?: string) => {
      if (isNewLoggingEnabled()) {
        stage.complete(result);
      } else {
        const timestamp = new Date().toISOString();
        console.log(`[${timestamp}] [INFO] ✅ STAGE COMPLETED`);
      }
    },
    
    fail: (error: string) => {
      if (isNewLoggingEnabled()) {
        stage.fail(error);
      } else {
        const timestamp = new Date().toISOString();
        console.error(`[${timestamp}] [ERROR] ❌ STAGE FAILED: ${error}`);
      }
    }
  },
  
  step: {
    start: (stepId: string, title: string, description?: string) => {
      if (isNewLoggingEnabled()) {
        step.start(stepId, title, description);
      } else {
        const timestamp = new Date().toISOString();
        console.log(`[${timestamp}] [INFO] ⚡ STEP ${stepId}: ${title}`);
      }
    },
    
    complete: (result?: string) => {
      if (isNewLoggingEnabled()) {
        step.complete(result);
      } else {
        const timestamp = new Date().toISOString();
        console.log(`[${timestamp}] [INFO] ✅ STEP COMPLETED`);
      }
    },
    
    fail: (error: string, hint?: string) => {
      if (isNewLoggingEnabled()) {
        step.fail(error, hint);
      } else {
        const timestamp = new Date().toISOString();
        console.error(`[${timestamp}] [ERROR] ❌ STEP FAILED: ${error}`);
      }
    },
    
    update: (message: string, metadata?: LogMetadata) => {
      if (isNewLoggingEnabled()) {
        step.update(message, metadata);
      } else {
        const timestamp = new Date().toISOString();
        console.log(`[${timestamp}] [INFO] ${message}`);
      }
    }
  }
};