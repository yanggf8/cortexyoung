#!/bin/bash

echo "🚀 Cortex Startup Script"
echo "========================"

# Configuration
DEFAULT_MODE="server"
DEFAULT_PORT="8765"

# Parse command line arguments
MODE="$DEFAULT_MODE"
PORT="$DEFAULT_PORT"
REBUILD=false
HEALTH_CHECK=true

while [[ $# -gt 0 ]]; do
    case $1 in
        --mode=*)
            MODE="${1#*=}"
            shift
            ;;
        --port=*)
            PORT="${1#*=}"
            shift
            ;;
        --rebuild)
            REBUILD=true
            shift
            ;;
        --no-health-check)
            HEALTH_CHECK=false
            shift
            ;;
        --help)
            echo ""
            echo "Usage: ./scripts/startup.sh [OPTIONS]"
            echo ""
            echo "Options:"
            echo "  --mode=MODE           Startup mode (server, demo, build)"
            echo "  --port=PORT           Server port (default: 8765)"
            echo "  --rebuild            Force full rebuild"
            echo "  --no-health-check    Skip pre-startup health check"
            echo "  --help               Show this help message"
            echo ""
            echo "Modes:"
            echo "  server               Start development server (default)"
            echo "  server:rebuild       Start server with forced rebuild"  
            echo "  demo                 Run indexing demo"
            echo "  demo:reindex         Run demo with full reindex"
            echo "  build                Build and start production server"
            echo ""
            echo "Examples:"
            echo "  ./scripts/startup.sh                    # Default server start"
            echo "  ./scripts/startup.sh --mode=demo        # Run indexing demo" 
            echo "  ./scripts/startup.sh --rebuild          # Force rebuild server"
            echo "  ./scripts/startup.sh --port=9000        # Custom port"
            exit 0
            ;;
        *)
            echo "❌ Unknown option: $1"
            echo "Use --help for usage information"
            exit 1
            ;;
    esac
done

# Pre-startup health check
if [ "$HEALTH_CHECK" = true ]; then
    echo ""
    echo "🩺 Pre-startup health check..."
    
    # Check for existing processes
    existing_processes=$(pgrep -f "npm.*server\|ts-node.*server\|node.*server\.js" 2>/dev/null)
    if [ ! -z "$existing_processes" ]; then
        echo "⚠️  Found existing server processes: $existing_processes"
        echo "🛑 Running cleanup first..."
        ./scripts/shutdown.sh
        echo ""
        echo "⏳ Waiting 3 seconds for cleanup to complete..."
        sleep 3
    else
        echo "✅ No conflicting processes found"
    fi
    
    # Check port availability
    if command -v lsof >/dev/null 2>&1; then
        port_usage=$(lsof -ti:$PORT 2>/dev/null)
        if [ ! -z "$port_usage" ]; then
            echo "⚠️  Port $PORT is in use by process: $port_usage"
            echo "🔧 Attempting to free port..."
            kill -TERM $port_usage 2>/dev/null
            sleep 2
        else
            echo "✅ Port $PORT is available"
        fi
    fi
    
    # Quick storage health check
    echo "📊 Storage health check..."
    if npm run --silent storage:status > /tmp/cortex_health.log 2>&1; then
        echo "✅ Storage system healthy"
    else
        echo "⚠️  Storage check completed with warnings (check logs)"
    fi
fi

echo ""
echo "🚀 Starting Cortex in '$MODE' mode..."

# Set environment variables
export PORT="$PORT"
export LOG_FILE="logs/cortex-server.log"

# Create logs directory if it doesn't exist
mkdir -p logs

# Determine npm command based on mode and options
case "$MODE" in
    "server")
        if [ "$REBUILD" = true ]; then
            NPM_COMMAND="npm run server:rebuild"
        else
            NPM_COMMAND="npm run server"
        fi
        ;;
    "server:rebuild")
        NPM_COMMAND="npm run server:rebuild"
        ;;
    "demo")
        if [ "$REBUILD" = true ]; then
            NPM_COMMAND="npm run demo:reindex"
        else
            NPM_COMMAND="npm run demo"
        fi
        ;;
    "demo:reindex")
        NPM_COMMAND="npm run demo:reindex"
        ;;
    "build")
        echo "🔧 Building project..."
        npm run build
        if [ $? -eq 0 ]; then
            echo "✅ Build successful"
            NPM_COMMAND="npm start"
        else
            echo "❌ Build failed"
            exit 1
        fi
        ;;
    *)
        echo "❌ Unknown mode: $MODE"
        echo "Valid modes: server, server:rebuild, demo, demo:reindex, build"
        exit 1
        ;;
esac

echo "💻 Command: $NPM_COMMAND"
echo "🌐 Port: $PORT"
echo "📝 Logs: $LOG_FILE"
echo ""

# Start the server in background and capture PID for server modes
if [[ "$MODE" == "server"* ]] || [[ "$MODE" == "build" ]]; then
    echo "🎯 Starting server..."
    echo "   Health endpoint: http://localhost:$PORT/health"
    echo "   Status endpoint: http://localhost:$PORT/status"
    echo ""
    echo "⏹️  Press Ctrl+C to stop gracefully"
    echo "⚠️  Or use: ./scripts/shutdown.sh for complete cleanup"
    echo ""
    
    # Run the command
    $NPM_COMMAND
else
    echo "🎯 Starting demo/indexing..."
    echo "⏹️  Press Ctrl+C to stop"
    echo ""
    
    # Run the command  
    $NPM_COMMAND
fi

# Capture exit code
EXIT_CODE=$?

echo ""
if [ $EXIT_CODE -eq 0 ]; then
    echo "✅ Process completed successfully"
elif [ $EXIT_CODE -eq 130 ]; then
    echo "⏹️  Process stopped by user (Ctrl+C)"
else
    echo "⚠️  Process exited with code: $EXIT_CODE"
fi

echo "🎯 Startup script complete!"