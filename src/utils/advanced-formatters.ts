/**
 * Advanced Data Formatters for Enhanced Console Logger
 * Provides structured data formatting, templates, and visual components
 */

// Color utilities - copied from console-logger for independence
const isColorSupported = (): boolean => {
  if (process.env.NO_COLOR) return false;
  if (process.env.FORCE_COLOR) return true;
  if (!process.stdout.isTTY) return false;
  return true;
};

// ANSI color codes
const colors = {
  reset: '\x1b[0m',
  bright: '\x1b[1m',
  dim: '\x1b[2m',
  red: '\x1b[91m',
  green: '\x1b[92m',
  yellow: '\x1b[93m',
  blue: '\x1b[94m',
  magenta: '\x1b[95m',
  cyan: '\x1b[96m',
  white: '\x1b[97m',
  gray: '\x1b[90m',
  blink: '\x1b[5m',
  underline: '\x1b[4m',
} as const;

const colorize = (text: string, color: keyof typeof colors): string => {
  if (!isColorSupported()) return text;
  return `${colors[color]}${text}${colors.reset}`;
};

// Re-export color utilities for advanced formatters
export { colorize, isColorSupported };

// Color mapping for data visualization - maps to base ANSI colors
const colorMap = {
  // Data types
  string: 'green',
  number: 'yellow', 
  boolean: 'magenta',
  null: 'gray',
  undefined: 'gray',
  
  // Status indicators
  success: 'green',
  warning: 'yellow',
  error: 'red',
  info: 'cyan',
  gray: 'gray',
  
  // UI elements
  border: 'gray',
  header: 'white',
  accent: 'cyan',
} as const;

export type DataColorKey = keyof typeof colorMap;
type ColorKey = keyof typeof colors;

// Helper to map data colors to actual ANSI colors
const mapColor = (dataColor: DataColorKey): ColorKey => {
  return colorMap[dataColor] as ColorKey;
};

// Progress bar configurations
export interface ProgressConfig {
  width?: number;
  filled?: string;
  empty?: string;
  brackets?: [string, string];
  showPercentage?: boolean;
  showNumbers?: boolean;
}

const defaultProgressConfig: Required<ProgressConfig> = {
  width: 20,
  filled: '█',
  empty: '░',
  brackets: ['[', ']'],
  showPercentage: true,
  showNumbers: false,
};

// Table formatting interfaces
export interface TableColumn {
  key: string;
  title: string;
  width?: number;
  align?: 'left' | 'center' | 'right';
  color?: DataColorKey;
}

export interface TableData {
  [key: string]: any;
}

export interface TableConfig {
  columns: TableColumn[];
  data: TableData[];
  border?: boolean;
  header?: boolean;
  maxWidth?: number;
}

// JSON formatting options
export interface JsonFormatConfig {
  indent?: number;
  colorize?: boolean;
  maxDepth?: number;
  compact?: boolean;
}

// List formatting options
export interface ListConfig {
  bullet?: string;
  indent?: number;
  numbered?: boolean;
  color?: DataColorKey;
}

/**
 * Format a progress bar
 */
export const formatProgress = (
  current: number,
  total: number,
  config: ProgressConfig = {}
): string => {
  const cfg = { ...defaultProgressConfig, ...config };
  const percentage = Math.min(100, Math.max(0, (current / total) * 100));
  const filled = Math.round((percentage / 100) * cfg.width);
  const empty = cfg.width - filled;
  
  const bar = cfg.filled.repeat(filled) + cfg.empty.repeat(empty);
  const coloredBar = colorize(bar, filled === cfg.width ? 'green' : 'cyan');
  
  let result = `${cfg.brackets[0]}${coloredBar}${cfg.brackets[1]}`;
  
  if (cfg.showPercentage) {
    const percent = colorize(`${percentage.toFixed(1)}%`, 'yellow');
    result += ` ${percent}`;
  }
  
  if (cfg.showNumbers) {
    const numbers = colorize(`${current}/${total}`, 'gray');
    result += ` ${numbers}`;
  }
  
  return result;
};

/**
 * Format data as a table
 */
export const formatTable = (config: TableConfig): string[] => {
  if (!config.data.length) return ['No data available'];
  
  const lines: string[] = [];
  const { columns, data, border = true, header = true, maxWidth = 120 } = config;
  
  // Calculate column widths
  const widths = columns.map(col => {
    const specifiedWidth = col.width;
    if (specifiedWidth) return specifiedWidth;
    
    const contentWidth = Math.max(
      col.title.length,
      ...data.map(row => String(row[col.key] || '').length)
    );
    return Math.min(contentWidth, Math.floor(maxWidth / columns.length) - 3);
  });
  
  // Helper to format cell content
  const formatCell = (content: string, width: number, align: string, color?: DataColorKey) => {
    const truncated = content.length > width ? content.substring(0, width - 3) + '...' : content;
    let padded: string;
    
    switch (align) {
      case 'center':
        const totalPad = width - truncated.length;
        const leftPad = Math.floor(totalPad / 2);
        const rightPad = totalPad - leftPad;
        padded = ' '.repeat(leftPad) + truncated + ' '.repeat(rightPad);
        break;
      case 'right':
        padded = truncated.padStart(width);
        break;
      default:
        padded = truncated.padEnd(width);
    }
    
    return color ? colorize(padded, mapColor(color)) : padded;
  };
  
  // Create border line
  const borderLine = border 
    ? '├' + widths.map(w => '─'.repeat(w + 2)).join('┼') + '┤'
    : '';
    
  // Top border
  if (border) {
    lines.push('┌' + widths.map(w => '─'.repeat(w + 2)).join('┬') + '┐');
  }
  
  // Header
  if (header) {
    const headerCells = columns.map((col, i) => 
      formatCell(col.title, widths[i], col.align || 'left', 'header')
    );
    lines.push('│ ' + headerCells.join(' │ ') + ' │');
    
    if (border) {
      lines.push(borderLine);
    }
  }
  
  // Data rows
  data.forEach((row, rowIndex) => {
    const cells = columns.map((col, i) => {
      const value = row[col.key];
      const stringValue = value === null ? 'null' : 
                         value === undefined ? 'undefined' : 
                         String(value);
      
      // Auto-detect color based on data type
      let cellColor = col.color;
      if (!cellColor && isColorSupported()) {
        if (typeof value === 'string') cellColor = 'string';
        else if (typeof value === 'number') cellColor = 'number';
        else if (typeof value === 'boolean') cellColor = 'boolean';
        else if (value === null || value === undefined) cellColor = 'null';
      }
      
      return formatCell(stringValue, widths[i], col.align || 'left', cellColor);
    });
    
    lines.push('│ ' + cells.join(' │ ') + ' │');
  });
  
  // Bottom border
  if (border) {
    lines.push('└' + widths.map(w => '─'.repeat(w + 2)).join('┴') + '┘');
  }
  
  return lines;
};

/**
 * Format JSON data with syntax highlighting
 */
export const formatJson = (
  data: any,
  config: JsonFormatConfig = {}
): string[] => {
  const { indent = 2, colorize: shouldColorize = true, maxDepth = 10, compact = false } = config;
  
  const formatValue = (value: any, depth: number = 0): string => {
    if (depth > maxDepth) {
      return colorize('[max depth reached]', 'gray');
    }
    
    const indentStr = compact ? '' : ' '.repeat(depth * indent);
    const nextIndentStr = compact ? '' : ' '.repeat((depth + 1) * indent);
    
    if (value === null) {
      return shouldColorize ? colorize('null', mapColor('null')) : 'null';
    }
    
    if (value === undefined) {
      return shouldColorize ? colorize('undefined', mapColor('undefined')) : 'undefined';
    }
    
    if (typeof value === 'string') {
      const escaped = JSON.stringify(value);
      return shouldColorize ? colorize(escaped, mapColor('string')) : escaped;
    }
    
    if (typeof value === 'number') {
      return shouldColorize ? colorize(String(value), mapColor('number')) : String(value);
    }
    
    if (typeof value === 'boolean') {
      return shouldColorize ? colorize(String(value), mapColor('boolean')) : String(value);
    }
    
    if (Array.isArray(value)) {
      if (value.length === 0) return '[]';
      if (compact) {
        return '[' + value.map(item => formatValue(item, depth + 1)).join(', ') + ']';
      }
      
      const items = value.map(item => 
        nextIndentStr + formatValue(item, depth + 1)
      );
      return '[\n' + items.join(',\n') + '\n' + indentStr + ']';
    }
    
    if (typeof value === 'object') {
      const keys = Object.keys(value);
      if (keys.length === 0) return '{}';
      
      if (compact) {
        const pairs = keys.map(key => 
          `${JSON.stringify(key)}: ${formatValue(value[key], depth + 1)}`
        );
        return '{' + pairs.join(', ') + '}';
      }
      
      const pairs = keys.map(key => {
        const keyStr = shouldColorize ? colorize(`"${key}"`, mapColor('accent')) : `"${key}"`;
        return `${nextIndentStr}${keyStr}: ${formatValue(value[key], depth + 1)}`;
      });
      return '{\n' + pairs.join(',\n') + '\n' + indentStr + '}';
    }
    
    return String(value);
  };
  
  return formatValue(data).split('\n');
};

/**
 * Format a list with bullets or numbers
 */
export const formatList = (
  items: string[],
  config: ListConfig = {}
): string[] => {
  const { bullet = '•', indent = 2, numbered = false, color } = config;
  
  return items.map((item, index) => {
    const marker = numbered ? `${index + 1}.` : bullet;
    const coloredMarker = color ? colorize(marker, mapColor(color)) : marker;
    const indentStr = ' '.repeat(indent);
    
    return `${indentStr}${coloredMarker} ${item}`;
  });
};

/**
 * Create a separator line
 */
export const formatSeparator = (
  width: number = 50,
  char: string = '─',
  color: DataColorKey = 'border'
): string => {
  return colorize(char.repeat(width), mapColor(color));
};

/**
 * Format a key-value pairs section
 */
export const formatKeyValue = (
  data: Record<string, any>,
  config: { 
    keyColor?: DataColorKey;
    valueColor?: DataColorKey;
    separator?: string;
    indent?: number;
  } = {}
): string[] => {
  const { keyColor = 'accent', valueColor, separator = ':', indent = 0 } = config;
  const indentStr = ' '.repeat(indent);
  
  return Object.entries(data).map(([key, value]) => {
    const coloredKey = colorize(key, mapColor(keyColor));
    const stringValue = String(value);
    const coloredValue = valueColor ? colorize(stringValue, mapColor(valueColor)) : stringValue;
    
    return `${indentStr}${coloredKey}${separator} ${coloredValue}`;
  });
};

/**
 * Create a box around text
 */
export const formatBox = (
  content: string[],
  config: {
    title?: string;
    padding?: number;
    borderColor?: DataColorKey;
    titleColor?: DataColorKey;
  } = {}
): string[] => {
  const { title, padding = 1, borderColor = 'border', titleColor = 'header' } = config;
  
  const maxWidth = Math.max(
    ...content.map(line => line.length),
    title ? title.length + 4 : 0
  );
  const width = maxWidth + (padding * 2) + 2;
  
  const lines: string[] = [];
  
  // Top border
  if (title) {
    const titlePadding = Math.max(0, width - title.length - 4);
    const leftPad = Math.floor(titlePadding / 2);
    const rightPad = titlePadding - leftPad;
    const coloredTitle = colorize(title, mapColor(titleColor));
    lines.push(colorize('┌─', mapColor(borderColor)) + ' '.repeat(leftPad) + coloredTitle + ' '.repeat(rightPad) + colorize('─┐', mapColor(borderColor)));
  } else {
    lines.push(colorize('┌' + '─'.repeat(width - 2) + '┐', mapColor(borderColor)));
  }
  
  // Content
  content.forEach(line => {
    const padded = line.padEnd(maxWidth);
    lines.push(colorize('│', mapColor(borderColor)) + ' '.repeat(padding) + padded + ' '.repeat(padding) + colorize('│', mapColor(borderColor)));
  });
  
  // Bottom border
  lines.push(colorize('└' + '─'.repeat(width - 2) + '┘', mapColor(borderColor)));
  
  return lines;
};

/**
 * Format a status indicator
 */
export const formatStatus = (
  status: 'success' | 'warning' | 'error' | 'info' | 'pending',
  message: string
): string => {
  const indicators = {
    success: { emoji: '✅', color: 'success' as const },
    warning: { emoji: '⚠️', color: 'warning' as const },
    error: { emoji: '❌', color: 'error' as const },
    info: { emoji: 'ℹ️', color: 'info' as const },
    pending: { emoji: '⏳', color: 'gray' as const },
  };
  
  const indicator = indicators[status];
  const coloredMessage = colorize(message, mapColor(indicator.color));
  
  return `${indicator.emoji} ${coloredMessage}`;
};

// Template system for common logging patterns
export const templates = {
  // System status template
  systemStatus: (data: {
    uptime: string;
    memory: string;
    cpu: string;
    processes: number;
    status: 'healthy' | 'warning' | 'error';
  }) => {
    const statusColor: DataColorKey = data.status === 'healthy' ? 'success' : 
                                     data.status === 'warning' ? 'warning' : 'error';
    
    return formatBox([
      formatStatus(data.status === 'healthy' ? 'success' : data.status, `System ${data.status.toUpperCase()}`),
      '',
      ...formatKeyValue({
        'Uptime': data.uptime,
        'Memory': data.memory,
        'CPU': data.cpu,
        'Processes': data.processes
      })
    ], { title: 'System Status', titleColor: statusColor });
  },
  
  // Performance metrics template
  performanceMetrics: (data: {
    operation: string;
    duration: string;
    throughput?: string;
    memoryUsage?: string;
    cacheHitRate?: string;
  }) => {
    const kvData: Record<string, any> = {
      'Duration': data.duration,
    };
    
    if (data.throughput) kvData['Throughput'] = data.throughput;
    if (data.memoryUsage) kvData['Memory'] = data.memoryUsage;
    if (data.cacheHitRate) kvData['Cache Hit Rate'] = data.cacheHitRate;
    
    return formatBox([
      colorize(data.operation, mapColor('header')),
      '',
      ...formatKeyValue(kvData)
    ], { title: 'Performance', titleColor: 'accent' });
  },
  
  // Error details template
  errorDetails: (data: {
    error: string;
    code?: string;
    location?: string;
    suggestion?: string;
  }) => {
    const content = [
      formatStatus('error', data.error),
    ];
    
    if (data.code) content.push('', `Code: ${colorize(data.code, mapColor('warning'))}`);
    if (data.location) content.push(`Location: ${colorize(data.location, mapColor('info'))}`);
    if (data.suggestion) {
      content.push('', colorize('💡 Suggestion:', mapColor('accent')));
      content.push(`   ${data.suggestion}`);
    }
    
    return formatBox(content, { title: 'Error Details', titleColor: 'error' });
  }
};