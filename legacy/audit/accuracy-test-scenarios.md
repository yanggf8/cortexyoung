# ClaudeCat Accuracy Self-Audit Test Scenarios

## Common Claude Code Mistakes to Test Against

### Authentication Pattern Mistakes
1. **Wrong User Property**: Suggests `user.id` when project uses `req.user.userId`
2. **Wrong Token Location**: Suggests localStorage when project uses httpOnly cookies
3. **Wrong Auth Middleware**: Suggests custom middleware when project uses Passport.js
4. **Wrong Error Format**: Suggests `{error: "message"}` when project uses `{errors: [{message}]}`

### API Response Pattern Mistakes  
1. **Wrong Wrapper Format**: Suggests bare objects when project wraps in `{data: any}`
2. **Wrong Status Codes**: Suggests 400 when project uses 422 for validation
3. **Wrong Error Structure**: Suggests string errors when project uses structured errors
4. **Wrong Success Format**: Suggests `{success: true}` when project returns `{status: "ok"}`

### Error Handling Pattern Mistakes
1. **Wrong Catch Pattern**: Suggests try/catch when project uses global middleware
2. **Wrong Error Propagation**: Suggests throwing when project uses next(error)
3. **Wrong Logging Integration**: Suggests console.log when project uses structured logging
4. **Wrong Error Types**: Suggests generic Error when project uses custom error classes

## Test Methodology

### Step 1: Create Test Codebases
Create minimal Express.js projects with known patterns:
- **Project A**: req.user + httpOnly cookies + {data: any} responses + global error middleware
- **Project B**: req.context.user + JWT headers + bare responses + try/catch blocks  
- **Project C**: Mixed patterns (conflicting implementations)

### Step 2: Pattern Detection Tests
For each project:
1. Run ClaudeCat detection
2. Verify detected patterns match actual implementation
3. Test confidence scores are accurate
4. Check for false positives in comments/tests

### Step 3: False Positive Tests  
Create files with pattern-like text that should NOT be detected:
```javascript
// This comment mentions req.user but doesn't implement it
const mockUser = {req: {user: 'test'}}; // Test mock, not real pattern
/* TODO: Consider using req.user in the future */
```

### Step 4: Integration Pattern Tests
Test if ClaudeCat detects how patterns work together:
- Does auth error format match API error format?
- Do middleware patterns align with error handling?
- Are token validation and user property consistent?

### Step 5: Confidence Threshold Validation
Test different confidence scenarios:
- **90%+ confidence**: Should be rock solid, no false positives
- **60-80% confidence**: Should have uncertainty warnings
- **<60% confidence**: Should not generate guardrails

## Success Criteria

### Pattern Detection Accuracy
- **95%+ precision**: Detected patterns must be correct (no false positives)
- **85%+ recall**: Must detect patterns that exist (minimal false negatives)
- **Confidence calibration**: 90% confidence = 90% actual accuracy

### Mistake Prevention
- **Before ClaudeCat**: Claude Code makes X wrong suggestions per 10 queries
- **After ClaudeCat**: Claude Code makes <0.7X wrong suggestions per 10 queries (30% reduction)

### Real-World Validation
- Test on 5 different real Express.js codebases
- Measure suggestion accuracy improvement
- Validate patterns match actual development practices