#!/usr/bin/env ts-node

/**
 * Telemetry Dashboard - Command-line interface for analyzing Claude Code usage patterns
 * 
 * Usage: npm run telemetry:dashboard [--days=7] [--export]
 */

import { TelemetryAnalyzer, UsageInsights, MMRPresetAnalysis } from './telemetry-analyzer';
import { ContextEffectivenessMetrics } from './telemetry-schema';
import { log } from './logging-utils';
import * as fs from 'fs/promises';
import * as path from 'path';

interface DashboardOptions {
  days: number;
  export: boolean;
  repositoryPath: string;
}

export class TelemetryDashboard {
  private analyzer: TelemetryAnalyzer;

  constructor(repositoryPath: string) {
    this.analyzer = new TelemetryAnalyzer(repositoryPath);
  }

  async generateReport(options: DashboardOptions): Promise<void> {
    console.log('\n🔍 CLAUDE CODE CONTEXT EFFECTIVENESS DASHBOARD');
    console.log('='.repeat(60));
    
    try {
      const [insights, effectiveness] = await Promise.all([
        this.analyzer.analyzeUsagePatterns(options.days),
        this.analyzer.generateContextEffectivenessReport()
      ]);

      this.displayOverview(insights, effectiveness, options.days);
      this.displayMMRAnalysis(insights.mmr_preset_analysis);
      this.displayQualityTrends(insights.quality_trends);
      this.displayRecommendations(insights.optimization_recommendations);

      if (options.export) {
        await this.exportReport(insights, effectiveness, options.repositoryPath);
      }

    } catch (error) {
      console.error('❌ Failed to generate telemetry report:', error instanceof Error ? error.message : error);
    }
  }

  private displayOverview(insights: UsageInsights, effectiveness: ContextEffectivenessMetrics, days: number): void {
    console.log(`\n📊 OVERVIEW (Past ${days} days)`);
    console.log('-'.repeat(40));
    console.log(`Total Queries: ${insights.total_queries}`);
    console.log(`Avg Response Time: ${insights.avg_query_response_time.toFixed(0)}ms`);
    console.log(`Most Effective Preset: ${insights.most_effective_preset}`);
    console.log(`Peak Usage Hours: ${insights.peak_usage_hours.join(', ')}`);

    console.log('\n🎯 CONTEXT EFFECTIVENESS METRICS');
    console.log('-'.repeat(40));
    console.log(`Overall Quality Score: ${this.formatScore(effectiveness.context_quality_score)} ${this.getQualityIcon(effectiveness.context_quality_score)}`);
    console.log(`Follow-up Rate: ${this.formatPercentage(effectiveness.avg_follow_up_rate)} ${this.getFollowUpIcon(effectiveness.avg_follow_up_rate)}`);
    console.log(`Reference Match Rate: ${this.formatPercentage(effectiveness.avg_reference_match_rate)}`);
    console.log(`Token Waste Rate: ${this.formatPercentage(effectiveness.avg_token_waste_rate)} ${this.getWasteIcon(effectiveness.avg_token_waste_rate)}`);
    console.log(`Critical Set Coverage: ${this.formatPercentage(effectiveness.avg_critical_set_coverage)} ${this.getCoverageIcon(effectiveness.avg_critical_set_coverage)}`);
    console.log(`User Satisfaction: ${this.formatPercentage(effectiveness.avg_user_satisfaction)}`);

    console.log('\n📈 QUERY PATTERNS');
    console.log('-'.repeat(40));
    insights.common_query_patterns.forEach(pattern => {
      console.log(`• ${pattern}`);
    });
  }

  private displayMMRAnalysis(presetAnalysis: MMRPresetAnalysis[]): void {
    console.log('\n🎛️  MMR PRESET PERFORMANCE ANALYSIS');
    console.log('-'.repeat(50));
    
    if (presetAnalysis.length === 0) {
      console.log('No MMR preset data available yet.');
      return;
    }

    console.log('Ranking | Preset        | Usage | Quality | Follow-up | Coverage | Efficiency');
    console.log('-'.repeat(75));
    
    presetAnalysis.forEach(preset => {
      const rank = `#${preset.effectiveness_ranking}`.padEnd(7);
      const name = preset.preset.padEnd(13);
      const usage = preset.usage_count.toString().padEnd(5);
      const quality = this.formatScore(preset.avg_context_quality_score);
      const followUp = this.formatPercentage(preset.avg_follow_up_rate);
      const coverage = this.formatPercentage(preset.avg_critical_set_coverage);
      const efficiency = this.formatPercentage(preset.avg_token_efficiency);
      
      console.log(`${rank} | ${name} | ${usage} | ${quality} | ${followUp} | ${coverage} | ${efficiency}`);
    });

    console.log('\n🏆 TOP PERFORMING PRESET DETAILS');
    console.log('-'.repeat(40));
    const topPreset = presetAnalysis[0];
    if (topPreset) {
      console.log(`Best Preset: ${topPreset.preset}`);
      console.log(`Quality Score: ${this.formatScore(topPreset.avg_context_quality_score)}`);
      console.log(`Recommended for: ${topPreset.recommended_use_cases.join(', ')}`);
    }
  }

  private displayQualityTrends(trends: any[]): void {
    console.log('\n📊 QUALITY TRENDS');
    console.log('-'.repeat(40));
    
    if (trends.length === 0) {
      console.log('Insufficient data for trend analysis.');
      return;
    }

    console.log('Date       | Quality | Follow-up | Coverage | Waste');
    console.log('-'.repeat(50));
    
    trends.slice(-7).forEach(trend => { // Show last 7 days
      const date = trend.time_period.substring(5); // MM-DD
      const quality = this.formatScore(trend.context_quality_score);
      const followUp = this.formatPercentage(trend.follow_up_rate);
      const coverage = this.formatPercentage(trend.critical_set_coverage);
      const waste = this.formatPercentage(trend.token_waste_rate);
      
      console.log(`${date}     | ${quality} | ${followUp}   | ${coverage} | ${waste}`);
    });

    // Calculate trend direction
    if (trends.length >= 2) {
      const recent = trends[trends.length - 1];
      const previous = trends[trends.length - 2];
      const qualityTrend = recent.context_quality_score > previous.context_quality_score ? '📈' : 
                          recent.context_quality_score < previous.context_quality_score ? '📉' : '➡️';
      
      console.log(`\nTrend: Quality ${qualityTrend} ${this.formatTrendChange(recent.context_quality_score, previous.context_quality_score)}`);
    }
  }

  private displayRecommendations(recommendations: string[]): void {
    console.log('\n💡 OPTIMIZATION RECOMMENDATIONS');
    console.log('-'.repeat(40));
    
    if (recommendations.length === 0) {
      console.log('✅ No specific optimizations needed - system performing well!');
      return;
    }

    recommendations.forEach((rec, index) => {
      console.log(`${index + 1}. ${rec}`);
    });

    console.log('\n🎯 PRIORITY ACTIONS');
    console.log('-'.repeat(25));
    if (recommendations.length > 0) {
      console.log(`🔥 High Priority: ${recommendations[0]}`);
      if (recommendations.length > 1) {
        console.log(`⚡ Medium Priority: ${recommendations[1]}`);
      }
    }
  }

  private async exportReport(
    insights: UsageInsights, 
    effectiveness: ContextEffectivenessMetrics, 
    repositoryPath: string
  ): Promise<void> {
    const reportData = {
      generated_at: new Date().toISOString(),
      summary: {
        total_queries: insights.total_queries,
        context_quality_score: effectiveness.context_quality_score,
        most_effective_preset: insights.most_effective_preset,
        avg_response_time: insights.avg_query_response_time
      },
      effectiveness_metrics: effectiveness,
      usage_insights: insights,
      export_version: '1.0'
    };

    const reportDir = path.join(repositoryPath, '.cortex', 'reports');
    await fs.mkdir(reportDir, { recursive: true });
    
    const timestamp = new Date().toISOString().split('T')[0];
    const reportPath = path.join(reportDir, `context-effectiveness-${timestamp}.json`);
    
    await fs.writeFile(reportPath, JSON.stringify(reportData, null, 2));
    console.log(`\n💾 Report exported to: ${reportPath}`);
  }

  // Utility formatting methods
  private formatScore(score: number): string {
    return `${(score * 100).toFixed(1)}%`;
  }

  private formatPercentage(rate: number): string {
    return `${(rate * 100).toFixed(1)}%`;
  }

  private getQualityIcon(score: number): string {
    if (score > 0.8) return '🟢';
    if (score > 0.6) return '🟡';
    return '🔴';
  }

  private getFollowUpIcon(rate: number): string {
    if (rate < 0.2) return '🟢'; // Low follow-up is good
    if (rate < 0.4) return '🟡';
    return '🔴';
  }

  private getWasteIcon(rate: number): string {
    if (rate < 0.2) return '🟢'; // Low waste is good
    if (rate < 0.4) return '🟡';
    return '🔴';
  }

  private getCoverageIcon(coverage: number): string {
    if (coverage > 0.9) return '🟢';
    if (coverage > 0.7) return '🟡';
    return '🔴';
  }

  private formatTrendChange(current: number, previous: number): string {
    const change = ((current - previous) / previous) * 100;
    const sign = change > 0 ? '+' : '';
    return `${sign}${change.toFixed(1)}%`;
  }
}

// CLI interface
async function main() {
  const args = process.argv.slice(2);
  const options: DashboardOptions = {
    days: 7,
    export: false,
    repositoryPath: process.cwd()
  };

  // Parse command line arguments
  for (const arg of args) {
    if (arg.startsWith('--days=')) {
      options.days = parseInt(arg.split('=')[1]) || 7;
    } else if (arg === '--export') {
      options.export = true;
    } else if (arg.startsWith('--repo=')) {
      options.repositoryPath = arg.split('=')[1];
    }
  }

  const dashboard = new TelemetryDashboard(options.repositoryPath);
  await dashboard.generateReport(options);
}

// Run if called directly
if (require.main === module) {
  main().catch(error => {
    console.error('Dashboard error:', error);
    process.exit(1);
  });
}

export { TelemetryDashboard };