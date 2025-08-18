/**
 * Configuration System for Enhanced Console Logger
 * Provides profiles, themes, and customizable logging behavior
 */

// Define color keys compatible with both systems
type LoggerColorKey = 'red' | 'green' | 'yellow' | 'blue' | 'magenta' | 'cyan' | 'white' | 'gray';

// Base logging configuration
export interface LoggerConfig {
  // Feature flags
  enabled: boolean;
  colors: boolean;
  emojis: boolean;
  timestamps: boolean;
  metadata: boolean;
  
  // Format settings
  timestampFormat: 'iso' | 'short' | 'time' | 'relative';
  logLevel: 'debug' | 'info' | 'warn' | 'error' | 'silent';
  maxLineLength?: number;
  indentSize: number;
  
  // Stage/Step settings
  stageDelimiters: boolean;
  stepDelimiters: boolean;
  progressIndicators: boolean;
  autoTiming: boolean;
  
  // Output settings
  outputTarget: 'console' | 'file' | 'both';
  logFile?: string;
  bufferSize?: number;
  
  // Theme settings
  theme: LoggerTheme;
}

// Theme configuration for colors and styles
export interface LoggerTheme {
  name: string;
  colors: {
    timestamp: LoggerColorKey;
    stage: LoggerColorKey;
    step: LoggerColorKey;
    success: LoggerColorKey;
    warning: LoggerColorKey;
    error: LoggerColorKey;
    info: LoggerColorKey;
    metadata: LoggerColorKey;
    accent: LoggerColorKey;
    border: LoggerColorKey;
  };
  emojis: {
    stage: string;
    step: string;
    success: string;
    warning: string;
    error: string;
    info: string;
    ready: string;
  };
  borders: {
    stage: {
      top: string;
      bottom: string;
      char: string;
    };
    step: {
      top: string;
      bottom: string;
      char: string;
    };
  };
}

// Predefined themes
export const themes: Record<string, LoggerTheme> = {
  default: {
    name: 'Default',
    colors: {
      timestamp: 'gray',
      stage: 'cyan',
      step: 'cyan',
      success: 'success',
      warning: 'warning',
      error: 'error',
      info: 'info',
      metadata: 'gray',
      accent: 'accent',
      border: 'border',
    },
    emojis: {
      stage: '🚀',
      step: '⚡',
      success: '✅',
      warning: '⚠️',
      error: '❌',
      info: 'ℹ️',
      ready: '🎉',
    },
    borders: {
      stage: {
        top: '==========================================',
        bottom: '==========================================',
        char: '=',
      },
      step: {
        top: '------------------------------------------',
        bottom: '------------------------------------------',
        char: '-',
      },
    },
  },

  minimal: {
    name: 'Minimal',
    colors: {
      timestamp: 'gray',
      stage: 'white',
      step: 'white',
      success: 'success',
      warning: 'warning',
      error: 'error',
      info: 'info',
      metadata: 'gray',
      accent: 'white',
      border: 'gray',
    },
    emojis: {
      stage: '•',
      step: '→',
      success: '✓',
      warning: '!',
      error: '✗',
      info: 'i',
      ready: '✓',
    },
    borders: {
      stage: {
        top: '─────────────────────────────────────────',
        bottom: '─────────────────────────────────────────',
        char: '─',
      },
      step: {
        top: '  ┌─────────────────────────────────────',
        bottom: '  └─────────────────────────────────────',
        char: '─',
      },
    },
  },

  colorful: {
    name: 'Colorful',
    colors: {
      timestamp: 'gray',
      stage: 'header',
      step: 'accent',
      success: 'success',
      warning: 'warning',
      error: 'error',
      info: 'info',
      metadata: 'gray',
      accent: 'accent',
      border: 'accent',
    },
    emojis: {
      stage: '🌟',
      step: '🔸',
      success: '🎯',
      warning: '🔶',
      error: '🔥',
      info: '💡',
      ready: '🚀',
    },
    borders: {
      stage: {
        top: '╔════════════════════════════════════════╗',
        bottom: '╚════════════════════════════════════════╝',
        char: '═',
      },
      step: {
        top: '  ╭────────────────────────────────────╮',
        bottom: '  ╰────────────────────────────────────╯',
        char: '─',
      },
    },
  },

  monochrome: {
    name: 'Monochrome',
    colors: {
      timestamp: 'gray',
      stage: 'white',
      step: 'white',
      success: 'white',
      warning: 'white',
      error: 'white',
      info: 'white',
      metadata: 'gray',
      accent: 'white',
      border: 'gray',
    },
    emojis: {
      stage: '',
      step: '',
      success: '',
      warning: '',
      error: '',
      info: '',
      ready: '',
    },
    borders: {
      stage: {
        top: '==========================================',
        bottom: '==========================================',
        char: '=',
      },
      step: {
        top: '------------------------------------------',
        bottom: '------------------------------------------',
        char: '-',
      },
    },
  },
};

// Predefined configuration profiles
export const profiles: Record<string, LoggerConfig> = {
  development: {
    enabled: true,
    colors: true,
    emojis: true,
    timestamps: true,
    metadata: true,
    timestampFormat: 'short',
    logLevel: 'debug',
    indentSize: 2,
    stageDelimiters: true,
    stepDelimiters: true,
    progressIndicators: true,
    autoTiming: true,
    outputTarget: 'console',
    theme: themes.default,
  },

  production: {
    enabled: true,
    colors: true,
    emojis: false,
    timestamps: true,
    metadata: false,
    timestampFormat: 'iso',
    logLevel: 'info',
    maxLineLength: 120,
    indentSize: 2,
    stageDelimiters: false,
    stepDelimiters: false,
    progressIndicators: false,
    autoTiming: false,
    outputTarget: 'both',
    logFile: 'logs/cortex-server.log',
    theme: themes.minimal,
  },

  ci: {
    enabled: true,
    colors: false,
    emojis: false,
    timestamps: true,
    metadata: false,
    timestampFormat: 'iso',
    logLevel: 'info',
    maxLineLength: 100,
    indentSize: 2,
    stageDelimiters: false,
    stepDelimiters: false,
    progressIndicators: true,
    autoTiming: false,
    outputTarget: 'console',
    theme: themes.monochrome,
  },

  debug: {
    enabled: true,
    colors: true,
    emojis: true,
    timestamps: true,
    metadata: true,
    timestampFormat: 'relative',
    logLevel: 'debug',
    indentSize: 4,
    stageDelimiters: true,
    stepDelimiters: true,
    progressIndicators: true,
    autoTiming: true,
    outputTarget: 'both',
    logFile: 'logs/debug.log',
    bufferSize: 1000,
    theme: themes.colorful,
  },

  testing: {
    enabled: true,
    colors: true,
    emojis: true,
    timestamps: false,
    metadata: false,
    timestampFormat: 'short',
    logLevel: 'warn',
    indentSize: 2,
    stageDelimiters: false,
    stepDelimiters: false,
    progressIndicators: true,
    autoTiming: false,
    outputTarget: 'console',
    theme: themes.minimal,
  },

  silent: {
    enabled: false,
    colors: false,
    emojis: false,
    timestamps: false,
    metadata: false,
    timestampFormat: 'iso',
    logLevel: 'silent',
    indentSize: 0,
    stageDelimiters: false,
    stepDelimiters: false,
    progressIndicators: false,
    autoTiming: false,
    outputTarget: 'console',
    theme: themes.monochrome,
  },
};

// Configuration manager class
export class LoggerConfigManager {
  private config: LoggerConfig;
  private readonly configFile?: string;

  constructor(profile: string = 'development', configFile?: string) {
    this.config = this.loadProfile(profile);
    this.configFile = configFile;
    
    if (configFile) {
      this.loadFromFile();
    }
  }

  // Load a predefined profile
  loadProfile(profileName: string): LoggerConfig {
    if (!profiles[profileName]) {
      console.warn(`Logger profile '${profileName}' not found, using 'development'`);
      return { ...profiles.development };
    }
    return { ...profiles[profileName] };
  }

  // Load configuration from file
  loadFromFile(): void {
    if (!this.configFile) return;
    
    try {
      const fs = require('fs');
      const path = require('path');
      
      if (fs.existsSync(this.configFile)) {
        const configData = JSON.parse(fs.readFileSync(this.configFile, 'utf8'));
        this.config = { ...this.config, ...configData };
      }
    } catch (error) {
      console.warn(`Failed to load logger config from ${this.configFile}:`, error);
    }
  }

  // Save configuration to file
  saveToFile(): void {
    if (!this.configFile) return;
    
    try {
      const fs = require('fs');
      const path = require('path');
      
      // Ensure directory exists
      const dir = path.dirname(this.configFile);
      if (!fs.existsSync(dir)) {
        fs.mkdirSync(dir, { recursive: true });
      }
      
      fs.writeFileSync(this.configFile, JSON.stringify(this.config, null, 2));
    } catch (error) {
      console.error(`Failed to save logger config to ${this.configFile}:`, error);
    }
  }

  // Get current configuration
  getConfig(): LoggerConfig {
    return { ...this.config };
  }

  // Update configuration
  updateConfig(updates: Partial<LoggerConfig>): void {
    this.config = { ...this.config, ...updates };
    
    if (this.configFile) {
      this.saveToFile();
    }
  }

  // Switch theme
  setTheme(themeName: string): void {
    if (!themes[themeName]) {
      console.warn(`Theme '${themeName}' not found`);
      return;
    }
    
    this.config.theme = themes[themeName];
    
    if (this.configFile) {
      this.saveToFile();
    }
  }

  // Environment-based auto-configuration
  autoConfigureFromEnvironment(): void {
    // Detect environment
    const env = process.env.NODE_ENV || 'development';
    const ci = process.env.CI === 'true';
    const debug = process.env.DEBUG === 'true';
    const testing = process.env.NODE_ENV === 'test';
    
    // Auto-select profile
    let profileName = 'development';
    
    if (ci) {
      profileName = 'ci';
    } else if (testing) {
      profileName = 'testing';
    } else if (env === 'production') {
      profileName = 'production';
    } else if (debug) {
      profileName = 'debug';
    }
    
    this.config = this.loadProfile(profileName);
    
    // Environment-specific overrides
    if (process.env.NO_COLOR === '1') {
      this.config.colors = false;
    }
    
    if (process.env.DISABLE_EMOJIS === '1') {
      this.config.emojis = false;
    }
    
    if (process.env.LOG_LEVEL) {
      const level = process.env.LOG_LEVEL.toLowerCase();
      if (['debug', 'info', 'warn', 'error', 'silent'].includes(level)) {
        this.config.logLevel = level as LoggerConfig['logLevel'];
      }
    }
    
    if (process.env.LOG_FILE) {
      this.config.logFile = process.env.LOG_FILE;
      this.config.outputTarget = 'both';
    }
  }

  // Get available profiles
  static getAvailableProfiles(): string[] {
    return Object.keys(profiles);
  }

  // Get available themes
  static getAvailableThemes(): string[] {
    return Object.keys(themes);
  }

  // Create custom theme
  static createCustomTheme(name: string, baseTheme: string, overrides: Partial<LoggerTheme>): void {
    if (!themes[baseTheme]) {
      throw new Error(`Base theme '${baseTheme}' not found`);
    }
    
    themes[name] = {
      ...themes[baseTheme],
      ...overrides,
      name,
    };
  }

  // Create custom profile
  static createCustomProfile(name: string, baseProfile: string, overrides: Partial<LoggerConfig>): void {
    if (!profiles[baseProfile]) {
      throw new Error(`Base profile '${baseProfile}' not found`);
    }
    
    profiles[name] = {
      ...profiles[baseProfile],
      ...overrides,
    };
  }
}

// Global configuration instance
let globalConfig: LoggerConfigManager | null = null;

// Initialize global configuration
export function initializeLoggerConfig(profile?: string, configFile?: string): LoggerConfigManager {
  globalConfig = new LoggerConfigManager(profile, configFile);
  globalConfig.autoConfigureFromEnvironment();
  return globalConfig;
}

// Get global configuration
export function getLoggerConfig(): LoggerConfig {
  if (!globalConfig) {
    globalConfig = new LoggerConfigManager();
    globalConfig.autoConfigureFromEnvironment();
  }
  return globalConfig.getConfig();
}

// Update global configuration
export function updateLoggerConfig(updates: Partial<LoggerConfig>): void {
  if (!globalConfig) {
    globalConfig = new LoggerConfigManager();
  }
  globalConfig.updateConfig(updates);
}

// Environment detection utilities
export const environment = {
  isProduction: () => process.env.NODE_ENV === 'production',
  isDevelopment: () => process.env.NODE_ENV === 'development' || !process.env.NODE_ENV,
  isTesting: () => process.env.NODE_ENV === 'test',
  isCI: () => process.env.CI === 'true',
  isDebug: () => process.env.DEBUG === 'true',
  hasColors: () => process.env.NO_COLOR !== '1' && process.stdout.isTTY,
  isTTY: () => process.stdout.isTTY,
};