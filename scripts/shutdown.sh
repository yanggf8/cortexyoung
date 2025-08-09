#!/bin/bash

echo "🛑 Cortex Shutdown Script"
echo "========================"

# Function to check if processes exist
check_processes() {
    local pattern="$1"
    local description="$2"
    local count=$(pgrep -f "$pattern" | wc -l)
    if [ $count -gt 0 ]; then
        echo "   Found $count $description processes"
        return 0
    else
        echo "   No $description processes found"
        return 1
    fi
}

# Function to kill processes gracefully, then forcefully if needed
kill_processes() {
    local pattern="$1"
    local description="$2"
    
    echo "🔄 Stopping $description..."
    
    # Get process IDs
    local pids=$(pgrep -f "$pattern")
    
    if [ -z "$pids" ]; then
        echo "   ✅ No $description processes running"
        return 0
    fi
    
    echo "   📋 Found processes: $pids"
    
    # First attempt: graceful shutdown (SIGTERM)
    echo "   🤝 Sending SIGTERM (graceful shutdown)..."
    pkill -TERM -f "$pattern" 2>/dev/null
    sleep 3
    
    # Check if processes are still running
    local remaining=$(pgrep -f "$pattern")
    if [ -z "$remaining" ]; then
        echo "   ✅ $description stopped gracefully"
        return 0
    fi
    
    echo "   ⏱️  Some processes still running, waiting 2 more seconds..."
    sleep 2
    
    # Second attempt: force kill (SIGKILL)
    remaining=$(pgrep -f "$pattern")
    if [ ! -z "$remaining" ]; then
        echo "   💥 Force killing remaining processes: $remaining"
        pkill -KILL -f "$pattern" 2>/dev/null
        sleep 1
    fi
    
    # Final check
    remaining=$(pgrep -f "$pattern")
    if [ -z "$remaining" ]; then
        echo "   ✅ All $description processes stopped"
    else
        echo "   ⚠️  Warning: Some processes may still be running: $remaining"
    fi
}

echo ""
echo "🔍 Scanning for Cortex processes..."

# Check what's running before shutdown
echo ""
echo "📊 Current process status:"
check_processes "npm.*(server|demo)" "npm server/demo"
check_processes "ts-node.*(server|index)" "ts-node server/indexer" 
check_processes "node.*server\.js" "compiled server"
check_processes "node.*external-embedding-process" "embedding workers"
check_processes "cortex" "cortex-related"

echo ""
echo "🛑 Initiating shutdown sequence..."

# 1. Stop main server processes
kill_processes "npm.*server" "npm server"
kill_processes "ts-node.*server" "ts-node server"
kill_processes "node.*server\.js" "compiled server"

# 2. Stop demo/indexing processes  
kill_processes "npm.*demo" "npm demo"
kill_processes "ts-node.*index" "ts-node indexer"

# 3. Stop embedding worker processes
kill_processes "node.*external-embedding-process" "embedding workers"

# 4. Stop any remaining cortex processes
kill_processes "cortex" "cortex-related"

# 5. Final cleanup - any remaining npm/ts-node processes related to the project
echo ""
echo "🧹 Final cleanup..."
cd_path=$(pwd)
kill_processes "npm.*${cd_path##*/}" "project npm processes"
kill_processes "ts-node.*${cd_path##*/}" "project ts-node processes"

echo ""
echo "🔍 Post-shutdown verification..."

# Verify shutdown
all_clear=true

for pattern in "npm.*server" "ts-node.*server" "node.*server\.js" "npm.*demo" "ts-node.*index" "node.*external-embedding-process"; do
    remaining=$(pgrep -f "$pattern" 2>/dev/null)
    if [ ! -z "$remaining" ]; then
        echo "⚠️  Warning: Processes still running for pattern '$pattern': $remaining"
        all_clear=false
    fi
done

if [ "$all_clear" = true ]; then
    echo "✅ All Cortex processes successfully stopped"
    echo ""
    echo "💡 System is clean and ready for restart"
    echo "   Use: ./scripts/startup.sh or npm run server"
else
    echo "⚠️  Some processes may still be running"
    echo "   You may need to restart your terminal or reboot if issues persist"
fi

echo ""
echo "🎯 Shutdown complete!"