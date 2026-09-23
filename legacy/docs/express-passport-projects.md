# Express + Passport Ground Truth Dataset

## Selected Projects for Pattern Detection Accuracy Testing

This dataset contains 15 carefully selected Express + Passport projects from GitHub for establishing ground truth authentication patterns.

### Selection Criteria:
- Uses Express.js framework
- Implements Passport.js authentication
- Contains `app.use(passport.authenticate())` or similar patterns
- Active maintenance (updated 2024-2025)
- Sufficient stars/forks to indicate real-world usage
- Variety of authentication strategies (local, GitHub, Google, etc.)

### Project List:

#### 1. **jaredhanson/passport** (Official)
- **URL**: https://github.com/jaredhanson/passport
- **Description**: Official Passport.js repository with examples
- **Auth Patterns**: Multiple strategies, core middleware setup
- **Expected Patterns**: `app.use(passport.initialize())`, `app.use(passport.session())`

#### 2. **Vladimirbr/express-jwt-passport-local-mongoose-winston**
- **URL**: https://github.com/Vladimirbr/express-jwt-passport-local-mongoose-winston
- **Description**: Sample authorization using JWT, Passport, Mongoose
- **Auth Patterns**: passport-local, JWT tokens
- **Expected Patterns**: `passport.use(new LocalStrategy(...))`, `app.use(passport.initialize())`

#### 3. **manish0502/User-Authentication-in-Web-Apps**
- **URL**: https://github.com/manish0502/User-Authentication-in-Web-Apps
- **Description**: User Authentication in Web Apps (Passport.js, Node, Express) with projects
- **Auth Patterns**: Multiple authentication strategies
- **Expected Patterns**: `app.use(passport.authenticate(...))`, social auth setups

#### 4. **chill117/passport-lnurl-auth** (Examples)
- **URL**: https://github.com/chill117/passport-lnurl-auth/blob/master/examples/simple.js
- **Description**: Lightning Network URL authentication example
- **Auth Patterns**: Custom strategy implementation
- **Expected Patterns**: `app.use(passport.authenticate('lnurl-auth'))`

#### 5. **diplomatiegouvfr/applitutoriel-js**
- **URL**: https://github.com/diplomatiegouvfr/applitutoriel-js
- **Description**: French government tutorial application
- **Auth Patterns**: Local authentication with TypeScript
- **Expected Patterns**: `passport.authenticate("local", {...})` in TypeScript

#### 6-15. **Additional Projects from GitHub Topics**

From passport-js, passport-local, and express-passport topics:

#### 6. **High-Star NestJS + Passport Project** (333 stars)
- Features: NestJS, Angular, GraphQL, JWT, Facebook/Twitter/Google auth
- Expected Patterns: passport-local, passport-jwt, social strategies

#### 7. **MERN Stack Authentication** (71 stars)  
- Features: React, Express, Passport, MongoDB
- Expected Patterns: Multiple strategies, token-based auth

#### 8. **TypeScript Express Auth** (79 stars)
- Features: TypeORM, Express, Angular, NestJS SSR
- Expected Patterns: passport-local, passport-jwt in TypeScript

#### 9. **Passport Examples Multi-DB**
- Features: MySQL, Sequelize, MongoDB, Mongoose examples
- Expected Patterns: Various database integrations with Passport

#### 10. **Express Session Auth**
- Features: express-session integration with Passport
- Expected Patterns: Session-based authentication

#### 11. **Passport Facebook Integration**
- Features: Facebook OAuth with Express
- Expected Patterns: `passport.use(new FacebookStrategy(...))`

#### 12. **Passport GitHub Strategy Example**
- Features: GitHub OAuth implementation
- Expected Patterns: `passport.use(new GitHubStrategy(...))`

#### 13. **JWT + Passport Local**
- Features: JSON Web Token integration
- Expected Patterns: Combined JWT and session auth

#### 14. **Passport Google OAuth**
- Features: Google OAuth 2.0 strategy
- Expected Patterns: `passport.use(new GoogleStrategy(...))`

#### 15. **Express Boilerplate Auth**
- Features: Complete Express authentication boilerplate
- Expected Patterns: Multiple auth strategies, user management

## Authentication Patterns to Detect:

### Core Middleware Setup:
- `app.use(passport.initialize())`
- `app.use(passport.session())`
- `app.use(express.session(...))`

### Strategy Definitions:
- `passport.use(new LocalStrategy(...))`
- `passport.use(new GitHubStrategy(...))`
- `passport.use(new GoogleStrategy(...))`
- `passport.use(new FacebookStrategy(...))`

### Route Protection:
- `app.use('/protected', passport.authenticate(...))`
- `router.get('/login', passport.authenticate('local', ...))`
- `passport.authenticate('github', { scope: [...] })`

### User Object Access:
- `req.user` property usage
- `req.isAuthenticated()` method calls
- `req.login()` and `req.logout()` methods

## Manual Verification Process:

For each project:
1. Clone repository locally
2. Identify all authentication middleware usage
3. Document exact patterns found (file paths, line numbers)
4. Categorize by strategy type (local, OAuth, custom)
5. Note confidence level (clear vs ambiguous patterns)
6. Record any edge cases or unusual implementations

## Expected Timeline:
- Project selection and cloning: 1 week
- Manual verification: 3-4 weeks intensive work
- Documentation and categorization: 1 week
- **Total Phase 1**: 5-6 weeks

## Success Criteria:
- Perfect ground truth for 15 Express + Passport projects
- 100% of authentication patterns documented with evidence
- Clear categorization of pattern types and confidence levels
- Baseline for measuring ClaudeCat accuracy improvements