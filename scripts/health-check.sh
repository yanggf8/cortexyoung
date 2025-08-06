#!/bin/bash

echo "🩺 Cortex System Health Check"
echo "============================="

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Health check results
ISSUES=0
WARNINGS=0

# Function to print status
print_status() {
    local status=$1
    local message=$2
    case $status in
        "OK")
            echo -e "   ✅ ${GREEN}$message${NC}"
            ;;
        "WARNING")
            echo -e "   ⚠️  ${YELLOW}$message${NC}"
            ((WARNINGS++))
            ;;
        "ERROR")
            echo -e "   ❌ ${RED}$message${NC}"
            ((ISSUES++))
            ;;
        "INFO")
            echo -e "   ℹ️  ${BLUE}$message${NC}"
            ;;
    esac
}

echo ""
echo "🔍 1. Process Health Check"
echo "-------------------------"

# Check for running processes
server_processes=$(pgrep -f "npm.*server\|ts-node.*server\|node.*server\.js" 2>/dev/null | wc -l)
demo_processes=$(pgrep -f "npm.*demo\|ts-node.*index" 2>/dev/null | wc -l)
embedding_processes=$(pgrep -f "node.*external-embedding-process" 2>/dev/null | wc -l)

if [ $server_processes -gt 0 ]; then
    print_status "INFO" "Server processes running: $server_processes"
    
    # Check if server is responding
    if command -v curl >/dev/null 2>&1; then
        if curl -s http://localhost:8765/health >/dev/null 2>&1; then
            print_status "OK" "Server responding on port 8765"
        else
            print_status "WARNING" "Server process running but not responding on port 8765"
        fi
    else
        print_status "INFO" "curl not available, cannot test server response"
    fi
else
    print_status "INFO" "No server processes running"
fi

if [ $demo_processes -gt 0 ]; then
    print_status "INFO" "Demo/indexing processes running: $demo_processes"
else
    print_status "OK" "No demo processes running"
fi

if [ $embedding_processes -gt 0 ]; then
    print_status "WARNING" "Embedding worker processes still running: $embedding_processes (may indicate incomplete shutdown)"
    echo "         Run: ./scripts/shutdown.sh to clean up"
else
    print_status "OK" "No orphaned embedding processes"
fi

echo ""
echo "🗄️  2. Storage Health Check"
echo "-------------------------"

# Check storage status
if npm run --silent storage:status > /tmp/cortex_storage_health.log 2>&1; then
    # Parse storage status for key metrics
    if grep -q "Synchronized: ✅" /tmp/cortex_storage_health.log; then
        print_status "OK" "Storage layers synchronized"
    elif grep -q "Synchronized: ❌" /tmp/cortex_storage_health.log; then
        print_status "WARNING" "Storage layers not synchronized (auto-sync will handle)"
    fi
    
    # Check embeddings
    embeddings_line=$(grep "Embeddings:" /tmp/cortex_storage_health.log)
    if echo "$embeddings_line" | grep -q "Local ✅.*Global ✅"; then
        print_status "OK" "Embeddings available in both storages"
    elif echo "$embeddings_line" | grep -q "✅"; then
        print_status "WARNING" "Embeddings available in one storage location"
    else
        print_status "ERROR" "No embeddings found"
    fi
    
    # Check relationships
    relationships_line=$(grep "Relationships:" /tmp/cortex_storage_health.log)
    if echo "$relationships_line" | grep -q "Local ✅.*Global ✅"; then
        print_status "OK" "Relationships available in both storages"
    elif echo "$relationships_line" | grep -q "✅"; then
        print_status "WARNING" "Relationships available in one storage location"  
    else
        print_status "WARNING" "Relationships missing (will be rebuilt on startup)"
    fi
    
    # Check for chunks count
    chunks_count=$(grep -o "[0-9]\+ chunks" /tmp/cortex_storage_health.log | head -1 | grep -o "[0-9]\+")
    if [ ! -z "$chunks_count" ] && [ "$chunks_count" -gt 0 ]; then
        print_status "OK" "Found $chunks_count indexed code chunks"
    else
        print_status "WARNING" "No indexed code chunks found"
    fi
    
else
    print_status "ERROR" "Storage health check failed"
fi

echo ""
echo "⚡ 3. Performance Health Check"  
echo "----------------------------"

# Quick performance validation
if npm run --silent validate:performance > /tmp/cortex_perf_health.log 2>&1; then
    # Check test results
    if grep -q "Tests Passed: 4/4" /tmp/cortex_perf_health.log; then
        print_status "OK" "All performance tests passed"
    else
        print_status "WARNING" "Some performance tests failed"
    fi
    
    # Check storage performance
    storage_time=$(grep -o "Status Check: [0-9.]\+ms" /tmp/cortex_perf_health.log | grep -o "[0-9.]\+")
    if [ ! -z "$storage_time" ]; then
        if (( $(echo "$storage_time < 10" | bc -l 2>/dev/null || echo "0") )); then
            print_status "OK" "Storage operations fast ($storage_time ms)"
        else
            print_status "WARNING" "Storage operations slower than expected ($storage_time ms)"
        fi
    fi
    
    # Check cache loading
    cache_time=$(grep -o "Cache Detection: [0-9]\+ms" /tmp/cortex_perf_health.log | grep -o "[0-9]\+")
    if [ ! -z "$cache_time" ]; then
        if [ "$cache_time" -lt 5000 ]; then
            print_status "OK" "Cache loading efficient ($cache_time ms)"
        else
            print_status "WARNING" "Cache loading slower than expected ($cache_time ms)"
        fi
    fi
    
else
    print_status "WARNING" "Performance validation completed with warnings"
fi

echo ""
echo "🔧 4. System Resources"
echo "---------------------"

# Check available memory
if command -v free >/dev/null 2>&1; then
    memory_info=$(free -m | grep "Mem:")
    total_mem=$(echo $memory_info | awk '{print $2}')
    available_mem=$(echo $memory_info | awk '{print $7}')
    memory_usage_percent=$(( (total_mem - available_mem) * 100 / total_mem ))
    
    if [ $memory_usage_percent -lt 80 ]; then
        print_status "OK" "Memory usage: ${memory_usage_percent}% (${available_mem}MB available)"
    else
        print_status "WARNING" "High memory usage: ${memory_usage_percent}% (${available_mem}MB available)"
    fi
elif command -v vm_stat >/dev/null 2>&1; then
    # macOS memory check
    print_status "INFO" "Memory check available (macOS detected)"
else
    print_status "INFO" "Memory check not available on this system"
fi

# Check disk space for current directory
disk_usage=$(df -h . | tail -1 | awk '{print $5}' | sed 's/%//')
if [ "$disk_usage" -lt 90 ]; then
    print_status "OK" "Disk space usage: ${disk_usage}%"
else
    print_status "WARNING" "High disk usage: ${disk_usage}%"
fi

# Check Node.js version
if command -v node >/dev/null 2>&1; then
    node_version=$(node --version)
    print_status "OK" "Node.js version: $node_version"
else
    print_status "ERROR" "Node.js not found"
fi

# Check npm version
if command -v npm >/dev/null 2>&1; then
    npm_version=$(npm --version)
    print_status "OK" "npm version: $npm_version"
else
    print_status "ERROR" "npm not found"
fi

echo ""
echo "📊 Health Check Summary"
echo "======================"

if [ $ISSUES -eq 0 ] && [ $WARNINGS -eq 0 ]; then
    print_status "OK" "System is healthy! All checks passed."
elif [ $ISSUES -eq 0 ]; then
    print_status "WARNING" "System is mostly healthy with $WARNINGS warnings"
    echo ""
    echo "💡 Recommendations:"
    echo "   • Warnings are usually minor and often resolve automatically"
    echo "   • Consider running: npm run storage:sync if storage issues persist"
    echo "   • Run: ./scripts/shutdown.sh followed by ./scripts/startup.sh for a clean restart"
else
    print_status "ERROR" "System has $ISSUES critical issues and $WARNINGS warnings"
    echo ""
    echo "🔧 Troubleshooting:"
    echo "   • Run: ./scripts/shutdown.sh to clean up processes"
    echo "   • Try: npm run cache:clear-all for storage issues"
    echo "   • Check logs in: logs/cortex-server.log"
    echo "   • Restart with: ./scripts/startup.sh --rebuild"
fi

echo ""
echo "📝 Log files checked:"
echo "   • /tmp/cortex_storage_health.log - Storage status"
echo "   • /tmp/cortex_perf_health.log - Performance results"

if [ -f logs/cortex-server.log ]; then
    print_status "INFO" "Server logs available: logs/cortex-server.log"
fi

echo ""
echo "🎯 Health check complete!"

# Exit with appropriate code
if [ $ISSUES -gt 0 ]; then
    exit 1
elif [ $WARNINGS -gt 0 ]; then
    exit 2
else
    exit 0
fi