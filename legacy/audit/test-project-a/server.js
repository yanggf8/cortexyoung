// Test Project A: Standard Express.js with common patterns
const express = require('express');
const jwt = require('jsonwebtoken');
const app = express();

// Auth middleware - uses req.user pattern
const authenticateToken = (req, res, next) => {
  const token = req.cookies.authToken; // httpOnly cookie pattern
  
  if (!token) {
    return res.status(401).json({
      data: null,
      error: { message: 'Authentication required' }
    });
  }

  jwt.verify(token, process.env.JWT_SECRET, (err, decoded) => {
    if (err) {
      return res.status(403).json({
        data: null, 
        error: { message: 'Invalid token' }
      });
    }
    req.user = decoded; // Standard req.user pattern
    next();
  });
};

// API routes with {data: any} wrapper pattern
app.get('/api/profile', authenticateToken, (req, res) => {
  res.json({
    data: {
      id: req.user.id,
      email: req.user.email,
      role: req.user.role
    }
  });
});

app.post('/api/posts', authenticateToken, (req, res) => {
  const newPost = {
    id: Date.now(),
    title: req.body.title,
    author: req.user.id
  };
  
  res.status(201).json({
    data: newPost
  });
});

// Global error handling middleware
app.use((err, req, res, next) => {
  console.error(err.stack);
  res.status(500).json({
    data: null,
    error: { message: 'Internal server error' }
  });
});

module.exports = app;