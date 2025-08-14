import { MMRConfig } from './guarded-mmr-selector';
import { log, warn } from './logging-utils';
import * as fs from 'fs/promises';
import * as path from 'path';

export interface MMRConfigPreset {
  name: string;
  description: string;
  config: MMRConfig;
  useCase: string;
}

export class MMRConfigManager {
  private static readonly DEFAULT_PRESETS: MMRConfigPreset[] = [
    {
      name: 'balanced',
      description: 'Balanced relevance and diversity for general code analysis',
      config: {
        lambdaRelevance: 0.7,
        maxTokenBudget: 100000,
        tokenCushionPercent: 0.20,
        diversityMetric: 'semantic',
        minCriticalSetCoverage: 0.95
      },
      useCase: 'General code analysis and debugging tasks'
    },
    {
      name: 'high-relevance',
      description: 'Prioritize relevance over diversity for focused analysis',
      config: {
        lambdaRelevance: 0.9,
        maxTokenBudget: 100000,
        tokenCushionPercent: 0.15,
        diversityMetric: 'cosine',
        minCriticalSetCoverage: 0.98
      },
      useCase: 'Specific function analysis, bug investigation'
    },
    {
      name: 'high-diversity',
      description: 'Prioritize diversity for comprehensive codebase exploration',
      config: {
        lambdaRelevance: 0.4,
        maxTokenBudget: 120000,
        tokenCushionPercent: 0.25,
        diversityMetric: 'semantic',
        minCriticalSetCoverage: 0.85
      },
      useCase: 'Architecture review, refactoring planning, codebase exploration'
    },
    {
      name: 'memory-conservative',
      description: 'Smaller token budget for resource-constrained environments',
      config: {
        lambdaRelevance: 0.8,
        maxTokenBudget: 50000,
        tokenCushionPercent: 0.30,
        diversityMetric: 'jaccard',
        minCriticalSetCoverage: 0.90
      },
      useCase: 'Local development, limited context windows'
    },
    {
      name: 'enterprise',
      description: 'Large token budget for complex enterprise codebases',
      config: {
        lambdaRelevance: 0.6,
        maxTokenBudget: 200000,
        tokenCushionPercent: 0.15,
        diversityMetric: 'semantic',
        minCriticalSetCoverage: 0.95
      },
      useCase: 'Enterprise codebases, comprehensive analysis'
    }
  ];

  private configPath: string;
  private currentConfig: MMRConfig;

  constructor(repositoryPath?: string) {
    this.configPath = repositoryPath ? 
      path.join(repositoryPath, '.cortex', 'mmr-config.json') :
      path.join(process.cwd(), '.cortex', 'mmr-config.json');
    
    this.currentConfig = MMRConfigManager.DEFAULT_PRESETS[0].config; // Default to balanced
  }

  async loadConfig(): Promise<MMRConfig> {
    try {
      const configExists = await this.configFileExists();
      if (configExists) {
        const configData = await fs.readFile(this.configPath, 'utf-8');
        const savedConfig = JSON.parse(configData);
        
        // Validate and merge with defaults
        this.currentConfig = this.validateAndMergeConfig(savedConfig);
        log(`[MMRConfig] Loaded custom MMR configuration from ${this.configPath}`);
      } else {
        log(`[MMRConfig] Using default balanced MMR configuration`);
      }
    } catch (error) {
      warn(`[MMRConfig] Failed to load MMR config, using defaults error=${error instanceof Error ? error.message : error}`);
    }
    
    return this.currentConfig;
  }

  async saveConfig(config: Partial<MMRConfig>): Promise<void> {
    try {
      // Ensure config directory exists
      await fs.mkdir(path.dirname(this.configPath), { recursive: true });
      
      // Merge with current config
      const newConfig = { ...this.currentConfig, ...config };
      
      // Validate configuration
      const validatedConfig = this.validateAndMergeConfig(newConfig);
      
      // Save to file
      await fs.writeFile(this.configPath, JSON.stringify(validatedConfig, null, 2));
      
      this.currentConfig = validatedConfig;
      log(`[MMRConfig] Saved MMR configuration to ${this.configPath}`);
    } catch (error) {
      warn(`[MMRConfig] Failed to save MMR config error=${error instanceof Error ? error.message : error}`);
      throw error;
    }
  }

  async applyPreset(presetName: string): Promise<MMRConfig> {
    const preset = MMRConfigManager.DEFAULT_PRESETS.find(p => p.name === presetName);
    if (!preset) {
      throw new Error(`Unknown MMR preset: ${presetName}. Available: ${this.getAvailablePresets().join(', ')}`);
    }
    
    await this.saveConfig(preset.config);
    log(`[MMRConfig] Applied MMR preset "${preset.name}": ${preset.description}`);
    
    return this.currentConfig;
  }

  getAvailablePresets(): string[] {
    return MMRConfigManager.DEFAULT_PRESETS.map(p => p.name);
  }

  getPresetDetails(presetName?: string): MMRConfigPreset | MMRConfigPreset[] {
    if (presetName) {
      const preset = MMRConfigManager.DEFAULT_PRESETS.find(p => p.name === presetName);
      if (!preset) {
        throw new Error(`Unknown MMR preset: ${presetName}`);
      }
      return preset;
    }
    
    return MMRConfigManager.DEFAULT_PRESETS;
  }

  getCurrentConfig(): MMRConfig {
    return { ...this.currentConfig };
  }

  async resetToDefaults(): Promise<MMRConfig> {
    const defaultConfig = MMRConfigManager.DEFAULT_PRESETS[0].config;
    await this.saveConfig(defaultConfig);
    log(`[MMRConfig] Reset MMR configuration to defaults`);
    
    return this.currentConfig;
  }

  private async configFileExists(): Promise<boolean> {
    try {
      await fs.access(this.configPath);
      return true;
    } catch {
      return false;
    }
  }

  private validateAndMergeConfig(config: any): MMRConfig {
    const defaultConfig = MMRConfigManager.DEFAULT_PRESETS[0].config;
    
    return {
      lambdaRelevance: this.validateNumber(config.lambdaRelevance, defaultConfig.lambdaRelevance, 0, 1),
      maxTokenBudget: this.validateNumber(config.maxTokenBudget, defaultConfig.maxTokenBudget, 10000, 500000),
      tokenCushionPercent: this.validateNumber(config.tokenCushionPercent, defaultConfig.tokenCushionPercent, 0, 0.5),
      diversityMetric: this.validateDiversityMetric(config.diversityMetric, defaultConfig.diversityMetric),
      minCriticalSetCoverage: this.validateNumber(config.minCriticalSetCoverage, defaultConfig.minCriticalSetCoverage, 0, 1)
    };
  }

  private validateNumber(value: any, defaultValue: number, min: number, max: number): number {
    if (typeof value !== 'number' || isNaN(value) || value < min || value > max) {
      warn(`[MMRConfig] Invalid number value ${value}, using default ${defaultValue}`);
      return defaultValue;
    }
    return value;
  }

  private validateDiversityMetric(value: any, defaultValue: 'cosine' | 'jaccard' | 'semantic'): 'cosine' | 'jaccard' | 'semantic' {
    const validMetrics = ['cosine', 'jaccard', 'semantic'];
    if (!validMetrics.includes(value)) {
      warn(`[MMRConfig] Invalid diversity metric ${value}, using default ${defaultValue}`);
      return defaultValue;
    }
    return value;
  }

  generateConfigReport(): string {
    const config = this.currentConfig;
    const matchingPreset = MMRConfigManager.DEFAULT_PRESETS.find(preset =>
      JSON.stringify(preset.config) === JSON.stringify(config)
    );
    
    const report = [
      '📊 Current MMR Configuration',
      '============================',
      matchingPreset ? `Preset: ${matchingPreset.name} (${matchingPreset.description})` : 'Preset: Custom configuration',
      '',
      '🎯 Selection Parameters:',
      `  Relevance Weight: ${(config.lambdaRelevance * 100).toFixed(0)}% / Diversity Weight: ${((1 - config.lambdaRelevance) * 100).toFixed(0)}%`,
      `  Token Budget: ${config.maxTokenBudget.toLocaleString()} tokens (~${Math.floor(config.maxTokenBudget / 4).toLocaleString()} Claude tokens)`,
      `  Safety Cushion: ${(config.tokenCushionPercent * 100).toFixed(0)}%`,
      `  Diversity Metric: ${config.diversityMetric}`,
      `  Critical Set Coverage: ${(config.minCriticalSetCoverage * 100).toFixed(0)}%`,
      '',
      '⚙️ Configuration Impact:',
      this.generateConfigInsights(config)
    ];
    
    if (matchingPreset) {
      report.push('', `🎯 Use Case: ${matchingPreset.useCase}`);
    }
    
    return report.join('\n');
  }

  private generateConfigInsights(config: MMRConfig): string {
    const insights: string[] = [];
    
    if (config.lambdaRelevance >= 0.8) {
      insights.push('  • High relevance focus - excellent for targeted analysis');
    } else if (config.lambdaRelevance <= 0.5) {
      insights.push('  • High diversity focus - excellent for broad exploration');
    } else {
      insights.push('  • Balanced approach - good for general-purpose analysis');
    }
    
    if (config.maxTokenBudget >= 150000) {
      insights.push('  • Large token budget - supports comprehensive analysis');
    } else if (config.maxTokenBudget <= 60000) {
      insights.push('  • Conservative token budget - optimized for efficiency');
    }
    
    if (config.tokenCushionPercent >= 0.25) {
      insights.push('  • High safety margin - protects against token overflow');
    }
    
    return insights.join('\n');
  }
}

// Environment-based configuration override
export function createMMRConfigFromEnvironment(): Partial<MMRConfig> | null {
  const config: Partial<MMRConfig> = {};
  let hasOverrides = false;

  if (process.env.CORTEX_MMR_LAMBDA) {
    const lambda = parseFloat(process.env.CORTEX_MMR_LAMBDA);
    if (!isNaN(lambda) && lambda >= 0 && lambda <= 1) {
      config.lambdaRelevance = lambda;
      hasOverrides = true;
    }
  }

  if (process.env.CORTEX_MMR_TOKEN_BUDGET) {
    const budget = parseInt(process.env.CORTEX_MMR_TOKEN_BUDGET, 10);
    if (!isNaN(budget) && budget > 0) {
      config.maxTokenBudget = budget;
      hasOverrides = true;
    }
  }

  if (process.env.CORTEX_MMR_DIVERSITY_METRIC) {
    const metric = process.env.CORTEX_MMR_DIVERSITY_METRIC;
    if (['cosine', 'jaccard', 'semantic'].includes(metric)) {
      config.diversityMetric = metric as 'cosine' | 'jaccard' | 'semantic';
      hasOverrides = true;
    }
  }

  return hasOverrides ? config : null;
}