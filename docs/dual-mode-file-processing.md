# Dual-Mode File Processing Architecture

## Overview

Cortex uses a dual-mode approach for handling file changes in real-time, with separate processing paths for git-tracked and untracked files.

## Architecture

### Git-Tracked Files (Direct Processing)
- **Path**: File watcher → Direct indexer processing
- **No staging**: Bypasses staging manager entirely
- **Immediate processing**: Changes are indexed immediately
- **Use case**: New git-tracked files, modifications to existing tracked files

```typescript
if (isGitTracked) {
  // Direct processing path
  const semanticChange = await this.analyzeSemanticChange(filePath, content);
  
  if (semanticChange) {
    await this.processSemanticChange(semanticChange);
  } else {
    await this.indexer.handleFileChange(filePath, 'content');
  }
}
```

### Untracked Files (Staging System)
- **Path**: File watcher → Staging manager → Indexer processing
- **Staging**: Uses intelligent staging with limits and filtering
- **Managed processing**: Controlled through staging status
- **Use case**: New untracked files, temporary files, work-in-progress files

```typescript
else {
  // Staging system path
  const staged = await this.stagingManager.stageFile(filePath);
  if (staged) {
    // Process through staging system
    await this.processFileChange(filePath);
    this.stagingManager.markFileIndexed(filePath);
  }
}
```

## Benefits

1. **Clean Separation**: Git-tracked files don't pollute staging area
2. **Performance**: Direct processing for tracked files is faster
3. **Resource Management**: Staging limits only apply to untracked files
4. **Design Clarity**: Each file type has its appropriate processing path

## Startup vs Real-time

### Startup Indexing
- **Scope**: Git-tracked files only
- **Purpose**: Fast initial indexing of known important files
- **Method**: `git ls-files` scan

### Real-time Processing
- **Git-tracked**: Direct processing (no staging)
- **Untracked**: Staging system with intelligent filtering
- **Purpose**: Handle new files and modifications as they happen

This architecture ensures that the staging system (`getFilesNeedingIndex()`) only shows untracked files that actually need processing, preventing accumulation of git-tracked files in the staging area.