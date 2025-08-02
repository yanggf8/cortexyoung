#!/bin/bash
# Start the Cortex V2.1 Embedding Service

echo "🚀 Starting Cortex V2.1 Embedding Service..."

# Check if virtual environment exists
if [ ! -d "venv" ]; then
    echo "📦 Creating Python virtual environment..."
    python3 -m venv venv
fi

# Activate virtual environment
echo "🔧 Activating virtual environment..."
source venv/bin/activate

# Install dependencies
echo "📥 Installing dependencies..."
pip install -r requirements.txt

# Start the service
echo "🌟 Starting FastAPI service on http://127.0.0.1:8000"
python embedding_service.py