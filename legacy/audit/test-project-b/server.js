// Test Project B: Different patterns - req.context, JWT headers, bare responses, try/catch
const express = require('express');
const jwt = require('jsonwebtoken');
const app = express();

// Auth middleware - uses req.context.user pattern
const authenticateToken = (req, res, next) => {
  const authHeader = req.headers.authorization; // JWT in headers, not cookies
  const token = authHeader && authHeader.split(' ')[1];
  
  if (!token) {
    return res.status(401).json({
      error: 'Access token required',
      code: 'AUTH_REQUIRED'
    });
  }

  try {
    const decoded = jwt.verify(token, process.env.JWT_SECRET);
    req.context = { user: decoded }; // Different pattern: req.context.user
    next();
  } catch (err) {
    return res.status(403).json({
      error: 'Invalid access token', 
      code: 'AUTH_INVALID'
    });
  }
};

// API routes with bare response pattern (no wrapper)
app.get('/api/me', authenticateToken, (req, res) => {
  try {
    const userProfile = {
      id: req.context.user.id,
      username: req.context.user.username,
      permissions: req.context.user.permissions
    };
    
    res.json(userProfile); // Bare response, no {data: ...} wrapper
  } catch (error) {
    res.status(500).json({
      error: 'Failed to fetch profile',
      code: 'PROFILE_ERROR'
    });
  }
});

app.post('/api/items', authenticateToken, (req, res) => {
  try {
    const newItem = {
      id: generateId(),
      name: req.body.name,
      createdBy: req.context.user.id,
      createdAt: new Date()
    };
    
    res.status(201).json(newItem); // Bare response
  } catch (error) {
    res.status(422).json({
      error: 'Validation failed',
      details: error.message,
      code: 'VALIDATION_ERROR'
    });
  }
});

function generateId() {
  return Math.random().toString(36).substring(7);
}

module.exports = app;