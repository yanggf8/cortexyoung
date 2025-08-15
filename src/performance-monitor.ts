import { EventEmitter } from 'events';
import { performance } from 'perf_hooks';
import * as os from 'os';
import { log } from './logging-utils';

interface PerformanceMetric {
  timestamp: number;
  name: string;
  value: number;
  unit: string;
  category: 'memory' | 'timing' | 'throughput' | 'system';
  details?: any;
}

interface SystemMetrics {
  timestamp: number;
  memory: {
    heapUsed: number;
    heapTotal: number;
    external: number;
    rss: number;
    arrayBuffers: number;
  };
  system: {
    loadAverage: number[];
    freeMemory: number;
    totalMemory: number;
    uptime: number;
  };
  process: {
    cpuUsage: NodeJS.CpuUsage;
    pid: number;
    uptime: number;
  };
}

export class PerformanceMonitor extends EventEmitter {
  private metrics: PerformanceMetric[] = [];
  private intervals: NodeJS.Timeout[] = [];
  private startTime: number;
  private isMonitoring = false;

  constructor() {
    super();
    this.startTime = performance.now();
  }

  // Record a timing metric
  recordTiming(name: string, duration: number, details?: any): void {
    this.recordMetric({
      timestamp: Date.now(),
      name,
      value: duration,
      unit: 'ms',
      category: 'timing',
      details
    });
  }

  // Record memory usage
  recordMemory(name: string, bytes: number, details?: any): void {
    this.recordMetric({
      timestamp: Date.now(),
      name,
      value: bytes / 1024 / 1024, // Convert to MB
      unit: 'MB',
      category: 'memory',
      details
    });
  }

  // Record throughput metric
  recordThroughput(name: string, itemsPerSecond: number, details?: any): void {
    this.recordMetric({
      timestamp: Date.now(),
      name,
      value: itemsPerSecond,
      unit: 'items/sec',
      category: 'throughput',
      details
    });
  }

  // Record a generic metric
  recordMetric(metric: PerformanceMetric): void {
    this.metrics.push(metric);
    this.emit('metric', metric);
    
    // Keep only last 1000 metrics to prevent memory leaks
    if (this.metrics.length > 1000) {
      this.metrics = this.metrics.slice(-1000);
    }
  }

  // Start continuous monitoring
  startMonitoring(intervalMs: number = 5000): void {
    if (this.isMonitoring) return;
    
    this.isMonitoring = true;
    log(`📊 Performance monitoring started (${intervalMs}ms intervals)`);
    
    // Monitor system metrics
    const systemInterval = setInterval(() => {
      this.recordSystemMetrics();
    }, intervalMs);
    
    this.intervals.push(systemInterval);
  }

  // Stop monitoring
  stopMonitoring(): void {
    if (!this.isMonitoring) return;
    
    this.intervals.forEach(interval => clearInterval(interval));
    this.intervals = [];
    this.isMonitoring = false;
    
    log('📊 Performance monitoring stopped');
  }

  // Record current system metrics
  private recordSystemMetrics(): void {
    const memory = process.memoryUsage();
    const cpuUsage = process.cpuUsage();
    
    // Memory metrics
    this.recordMemory('heap-used', memory.heapUsed);
    this.recordMemory('heap-total', memory.heapTotal);
    this.recordMemory('external', memory.external);
    this.recordMemory('rss', memory.rss);
    
    // System metrics
    this.recordMetric({
      timestamp: Date.now(),
      name: 'cpu-load-1m',
      value: os.loadavg()[0],
      unit: 'load',
      category: 'system'
    });
    
    this.recordMetric({
      timestamp: Date.now(),
      name: 'free-memory',
      value: os.freemem() / 1024 / 1024,
      unit: 'MB',
      category: 'system'
    });
  }

  // Get system metrics snapshot
  getSystemMetrics(): SystemMetrics {
    const memory = process.memoryUsage();
    const cpuUsage = process.cpuUsage();
    
    return {
      timestamp: Date.now(),
      memory: {
        heapUsed: memory.heapUsed,
        heapTotal: memory.heapTotal,
        external: memory.external,
        rss: memory.rss,
        arrayBuffers: memory.arrayBuffers
      },
      system: {
        loadAverage: os.loadavg(),
        freeMemory: os.freemem(),
        totalMemory: os.totalmem(),
        uptime: os.uptime()
      },
      process: {
        cpuUsage,
        pid: process.pid,
        uptime: process.uptime()
      }
    };
  }

  // Create a timing wrapper
  time<T>(name: string, operation: () => Promise<T>): Promise<T>;
  time<T>(name: string, operation: () => T): T;
  time<T>(name: string, operation: () => T | Promise<T>): T | Promise<T> {
    const startTime = performance.now();
    const startMemory = process.memoryUsage();
    
    const finish = (result: T): T => {
      const endTime = performance.now();
      const endMemory = process.memoryUsage();
      const duration = endTime - startTime;
      const memoryDelta = endMemory.heapUsed - startMemory.heapUsed;
      
      this.recordTiming(name, duration, {
        memoryDelta: memoryDelta / 1024 / 1024, // MB
        startMemory: startMemory.heapUsed / 1024 / 1024,
        endMemory: endMemory.heapUsed / 1024 / 1024
      });
      
      return result;
    };
    
    const finishError = (): void => {
      const endTime = performance.now();
      const duration = endTime - startTime;
      this.recordTiming(name, duration, { error: true });
    };
    
    try {
      const result = operation();
      
      if (result instanceof Promise) {
        return result.then(
          (value: T) => finish(value),
          (error) => {
            finishError();
            throw error;
          }
        );
      } else {
        return finish(result);
      }
    } catch (error) {
      finishError();
      throw error;
    }
  }

  // Get metrics by category
  getMetricsByCategory(category: PerformanceMetric['category']): PerformanceMetric[] {
    return this.metrics.filter(m => m.category === category);
  }

  // Get metrics by name pattern
  getMetricsByName(namePattern: string): PerformanceMetric[] {
    const regex = new RegExp(namePattern, 'i');
    return this.metrics.filter(m => regex.test(m.name));
  }

  // Calculate average for a metric
  getAverageMetric(name: string, timeWindowMs?: number): number | null {
    const now = Date.now();
    const metrics = this.metrics.filter(m => {
      const matchesName = m.name === name;
      const inTimeWindow = !timeWindowMs || (now - m.timestamp) <= timeWindowMs;
      return matchesName && inTimeWindow;
    });
    
    if (metrics.length === 0) return null;
    
    const sum = metrics.reduce((acc, m) => acc + m.value, 0);
    return sum / metrics.length;
  }

  // Get performance summary
  getSummary(timeWindowMs?: number): {
    totalMetrics: number;
    categories: Record<string, number>;
    memoryStats: {
      current: number;
      peak: number;
      average: number;
    };
    timingStats: {
      totalOperations: number;
      averageTime: number;
      slowestOperation: string | null;
    };
  } {
    const now = Date.now();
    const relevantMetrics = timeWindowMs 
      ? this.metrics.filter(m => (now - m.timestamp) <= timeWindowMs)
      : this.metrics;
    
    const categories: Record<string, number> = {};
    relevantMetrics.forEach(m => {
      categories[m.category] = (categories[m.category] || 0) + 1;
    });
    
    const memoryMetrics = relevantMetrics.filter(m => m.category === 'memory');
    const timingMetrics = relevantMetrics.filter(m => m.category === 'timing');
    
    const memoryValues = memoryMetrics.map(m => m.value);
    const timingValues = timingMetrics.map(m => m.value);
    
    let slowestOperation: string | null = null;
    if (timingMetrics.length > 0) {
      const slowest = timingMetrics.reduce((prev, current) => 
        prev.value > current.value ? prev : current
      );
      slowestOperation = slowest.name;
    }
    
    return {
      totalMetrics: relevantMetrics.length,
      categories,
      memoryStats: {
        current: memoryValues.length > 0 ? memoryValues[memoryValues.length - 1] : 0,
        peak: memoryValues.length > 0 ? Math.max(...memoryValues) : 0,
        average: memoryValues.length > 0 ? memoryValues.reduce((a, b) => a + b, 0) / memoryValues.length : 0
      },
      timingStats: {
        totalOperations: timingMetrics.length,
        averageTime: timingValues.length > 0 ? timingValues.reduce((a, b) => a + b, 0) / timingValues.length : 0,
        slowestOperation
      }
    };
  }

  // Clear all metrics
  clearMetrics(): void {
    this.metrics = [];
    log('📊 Performance metrics cleared');
  }

  // Export metrics to JSON
  exportMetrics(): {
    startTime: number;
    duration: number;
    metrics: PerformanceMetric[];
    summary: ReturnType<PerformanceMonitor['getSummary']>;
  } {
    const now = performance.now();
    return {
      startTime: this.startTime,
      duration: now - this.startTime,
      metrics: [...this.metrics],
      summary: this.getSummary()
    };
  }
}

// Global instance for easy access
export const performanceMonitor = new PerformanceMonitor();