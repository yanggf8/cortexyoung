import { testIntegratedDetection } from './dist/integrated-pattern-detector.js';

// Test on current project (should have no Passport patterns)
testIntegratedDetection('.');