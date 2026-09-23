// False Positive Test File - Should NOT detect patterns from these

/**
 * This file contains pattern-like text that should NOT be detected
 * as actual implementation patterns by ClaudeCat
 */

// 1. Comments mentioning patterns
// TODO: We should use req.user for authentication in the future
// Consider implementing {data: any} wrapper for API responses
/* 
 * Authentication flow:
 * 1. Extract token from req.headers.authorization
 * 2. Verify token and set req.user = decoded
 * 3. Return error with format {error: "message"}
 */

// 2. Documentation strings
const authDocs = `
  Authentication patterns in Express.js:
  - req.user contains the authenticated user
  - Tokens can be in cookies or headers
  - Global error middleware catches unhandled errors
`;

// 3. Test mocks and fixtures
const mockRequest = {
  user: { id: 1, email: 'test@example.com' },
  context: { user: { id: 1 } }
};

const mockResponse = {
  json: (data) => ({ data: data }),
  status: (code) => ({ json: (data) => data })
};

// 4. Configuration examples
const authConfig = {
  userProperty: 'req.user', // This is a config value, not implementation
  tokenLocation: 'httpOnly cookie',
  errorFormat: '{data: null, error: {message: string}}'
};

// 5. Dead/commented code
/*
function oldAuthMiddleware(req, res, next) {
  req.user = getUser(req.token);
  next();
}
*/

// 6. String literals and templates
const errorMessage = 'Could not find req.user property';
const codeTemplate = `
  if (!req.user) {
    return res.status(401).json({data: null, error: 'Unauthorized'});
  }
`;

// 7. Logging statements
console.log('Setting req.user to:', user);
console.error('Error format should be {data: null, error: message}');

// 8. Variable names that contain pattern keywords
const reqUserValidator = (user) => user.id && user.email;
const dataWrapper = (payload) => ({ data: payload });
const errorHandler = (err) => ({ error: err.message });

// 9. Import/require statements with pattern keywords  
// const userMiddleware = require('./middleware/req-user-handler');
// import { dataWrapper } from './utils/response-wrapper';

// 10. Type definitions (if this were TypeScript)
/*
interface AuthenticatedRequest {
  user: User;
  context: { user: User };
}

type ApiResponse<T> = { data: T } | { error: string };
*/

module.exports = {
  // These exports contain pattern keywords but aren't implementations
  userPatterns: ['req.user', 'req.context.user', 'req.auth'],
  responsePatterns: ['{data: any}', 'bare objects', '{error: string}'],
  errorPatterns: ['global middleware', 'try/catch', 'next(error)']
};