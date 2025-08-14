# MMR Deployment Guide

## Overview

The Guarded MMR (Maximal Marginal Relevance) system optimizes context window usage for Cortex by intelligently selecting the most relevant and diverse code chunks while guaranteeing critical set inclusion.

## Environment Configuration

### Core MMR Settings

```bash
# Enable/disable MMR (default: enabled)
CORTEX_MMR_ENABLED=true

# Relevance vs Diversity balance (0.0-1.0, default: 0.7)
# 0.9 = high relevance focus, 0.3 = high diversity focus
CORTEX_MMR_LAMBDA=0.7

# Token budget (default: 100000 = ~25k Claude tokens)
CORTEX_MMR_TOKEN_BUDGET=100000

# Diversity calculation method (default: semantic)
# Options: cosine, jaccard, semantic
CORTEX_MMR_DIVERSITY_METRIC=semantic
```

### Performance Tuning

```bash
# Critical set coverage requirement (0.0-1.0, default: 0.95)
CORTEX_MMR_MIN_CRITICAL_COVERAGE=0.95

# Token safety cushion (0.0-0.5, default: 0.20)
CORTEX_MMR_TOKEN_CUSHION=0.20
```

### Configuration Presets

Use predefined configurations optimized for different scenarios:

```bash
# Apply via MMRConfigManager
npm run mmr:config balanced          # General purpose
npm run mmr:config high-relevance    # Focused analysis  
npm run mmr:config high-diversity    # Comprehensive exploration
npm run mmr:config memory-conservative # Resource constrained
npm run mmr:config enterprise        # Large codebases
```

## Production Deployment

### 1. Performance Targets

- **Selection Time**: < 100ms for up to 1000 candidate chunks
- **Memory Overhead**: < 50MB additional memory usage
- **Critical Set Coverage**: > 95% for queries with explicit file/function mentions
- **Token Efficiency**: 30%+ increase in relevant chunks per token

### 2. Monitoring & Metrics

MMR provides comprehensive metrics in search responses:

```typescript
{
  metadata: {
    mmr_metrics: {
      critical_set_coverage: 0.95,    // 95% of critical items included
      diversity_score: 0.73,          // 73% diversity achieved
      budget_utilization: 0.87,       // 87% of token budget used
      selection_time_ms: 45           // 45ms selection time
    }
  }
}
```

### 3. Health Checks

```bash
# Test MMR functionality
npm run test:mmr

# Performance benchmarks
npm run benchmark:mmr

# Configuration validation
npm run mmr:validate
```

### 4. Scaling Considerations

#### Small Deployments (< 100k files)
- Use `balanced` or `high-relevance` presets
- Token budget: 50,000-100,000
- Expected selection time: < 50ms

#### Medium Deployments (100k-500k files)  
- Use `balanced` preset
- Token budget: 100,000-150,000
- Expected selection time: < 100ms

#### Large Enterprise (> 500k files)
- Use `enterprise` preset
- Token budget: 150,000-200,000
- Expected selection time: < 200ms
- Consider horizontal scaling

## Integration Checklist

### ✅ Pre-Deployment

- [ ] Environment variables configured
- [ ] MMR test suite passes
- [ ] Performance benchmarks meet targets  
- [ ] Configuration presets tested
- [ ] Monitoring dashboard updated

### ✅ Post-Deployment

- [ ] MMR metrics appearing in search responses
- [ ] Selection times within targets
- [ ] Critical set coverage > 95%
- [ ] Token utilization optimal
- [ ] No performance regressions

### ✅ Ongoing Monitoring

- [ ] Track selection time trends
- [ ] Monitor critical set coverage
- [ ] Analyze diversity score patterns
- [ ] Review token budget utilization
- [ ] Performance regression detection

## Troubleshooting

### Common Issues

#### 1. High Selection Times (> 100ms)
**Cause**: Large candidate sets or complex diversity calculations
**Solution**: 
- Reduce `CORTEX_MMR_TOKEN_BUDGET` 
- Switch to `cosine` diversity metric
- Use `memory-conservative` preset

#### 2. Low Critical Set Coverage (< 90%)
**Cause**: Query parsing missing important identifiers
**Solution**:
- Review critical set extraction logic
- Adjust `CORTEX_MMR_MIN_CRITICAL_COVERAGE`
- Check for typos in file/function names

#### 3. Poor Diversity Scores (< 0.3)
**Cause**: Over-emphasis on relevance
**Solution**:
- Reduce `CORTEX_MMR_LAMBDA` (try 0.5-0.6)
- Switch to `high-diversity` preset
- Use `semantic` diversity metric

#### 4. Token Budget Exceeded
**Cause**: Critical set too large for budget
**Solution**:
- Increase `CORTEX_MMR_TOKEN_BUDGET`
- Increase `CORTEX_MMR_TOKEN_CUSHION`
- Review query specificity

#### 5. MMR Not Activating
**Cause**: Candidate count below threshold
**Solution**:
- MMR only activates when candidates > max_chunks
- Increase candidate pool via relationship traversal
- Lower max_chunks parameter

### Debug Mode

Enable detailed MMR logging:

```bash
export CORTEX_DEBUG_MMR=true
npm run server
```

This provides:
- Critical set extraction details
- Selection step-by-step logging
- Performance timing breakdowns
- Token calculation insights

### Performance Profiling

Run comprehensive performance analysis:

```bash
# Full MMR test suite with performance metrics
npm run test:mmr:performance

# Scalability benchmark (10 to 2000 chunks)
npm run benchmark:mmr:scalability

# Memory usage analysis
npm run profile:mmr:memory
```

## Advanced Configuration

### Custom Diversity Metrics

Extend diversity calculation by implementing custom metrics:

```typescript
// In guarded-mmr-selector.ts
private calculateCustomSimilarity(chunk1: CodeChunk, chunk2: CodeChunk): number {
  // Custom similarity logic here
  return similarityScore;
}
```

### Dynamic Budget Adjustment

Implement adaptive token budgets based on query complexity:

```typescript
const dynamicBudget = baseTokenBudget * (1 + queryComplexityFactor);
```

### Critical Set Extensions

Enhance critical set extraction for domain-specific identifiers:

```typescript
// Custom patterns for specific frameworks/libraries
const frameworkPatterns = /\b(React|Vue|Angular|Express)\w*/gi;
```

## Security Considerations

### 1. Input Validation
- Query length limits (max 10,000 characters)
- File path sanitization
- Symbol name validation

### 2. Resource Protection  
- Token budget limits (max 500,000)
- Selection time limits (max 5 seconds)
- Memory usage monitoring

### 3. Access Control
- Configuration file permissions (600)
- Environment variable security
- Log file access control

## Migration Guide

### From Legacy Search to MMR

1. **Phase 1**: Enable MMR with `balanced` preset
2. **Phase 2**: Monitor performance and adjust configuration
3. **Phase 3**: Optimize presets based on usage patterns
4. **Phase 4**: Enable advanced features (custom diversity metrics)

### Rollback Plan

1. Set `CORTEX_MMR_ENABLED=false`
2. System automatically falls back to traditional ranking
3. Zero downtime - no restart required
4. Monitor performance to ensure stable operation

---

## Support

For MMR-related issues:

1. Check logs for `[MMR]` prefixed messages
2. Run diagnostic suite: `npm run mmr:diagnose`
3. Review configuration: `npm run mmr:config:report`
4. Submit performance reports with reproduction steps

MMR significantly improves context window efficiency - proper configuration ensures optimal results!