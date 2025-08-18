import { log } from './logging-utils';
import * as fs from 'fs/promises';
import * as path from 'path';

interface FileUsageEvidence {
  filePath: string;
  isGitTracked: boolean;
  timesChanged: number;
  timesSearched: number;
  timesAccessed: number;
  userInteractionScore: number;
  firstSeen: Date;
  lastAccessed: Date;
  contentQuality: number;
  fileSize: number;
  extension: string;
}

interface EvidenceReport {
  totalFiles: number;
  gitTrackedFiles: number;
  nonGitFiles: number;
  valuableNonGitFiles: FileUsageEvidence[];
  topInteractionFiles: FileUsageEvidence[];
  recommendedPatterns: string[];
}

export class UsageEvidenceCollector {
  private usageData: Map<string, FileUsageEvidence> = new Map();
  private evidenceFile: string;
  private saveInterval: NodeJS.Timeout | null = null;

  constructor(repositoryPath: string) {
    this.evidenceFile = path.join(repositoryPath, '.cortex', 'usage-evidence.json');
    this.loadExistingEvidence();
    this.startPeriodicSave();
  }

  /**
   * Record when a file is changed during development
   */
  recordFileChange(filePath: string, isGitTracked: boolean, fileSize: number = 0): void {
    const evidence = this.getOrCreateEvidence(filePath);
    evidence.timesChanged++;
    evidence.isGitTracked = isGitTracked;
    evidence.lastAccessed = new Date();
    evidence.fileSize = fileSize;
    
    // Score based on git status and change frequency
    if (isGitTracked) {
      evidence.userInteractionScore += 2; // Git files get base score
    } else {
      evidence.userInteractionScore += 1; // Non-git files get lower base score
      
      // But if user keeps editing non-git files, they might be valuable
      if (evidence.timesChanged > 3) {
        evidence.userInteractionScore += 1; // Bonus for frequent non-git edits
      }
    }
    
    log(`[Evidence] File change: ${filePath} (git: ${isGitTracked}, score: ${evidence.userInteractionScore})`);
  }

  /**
   * Record when a file appears in search results and user interacts with it
   */
  recordSearchHit(filePath: string, wasClicked: boolean = true): void {
    const evidence = this.getOrCreateEvidence(filePath);
    evidence.timesSearched++;
    evidence.lastAccessed = new Date();
    
    if (wasClicked) {
      // Strong evidence: user found this file useful in search
      evidence.userInteractionScore += 3;
      evidence.timesAccessed++;
      
      log(`[Evidence] Search hit: ${filePath} (clicked, score: ${evidence.userInteractionScore})`);
    } else {
      // Weak evidence: file appeared in search but wasn't clicked
      evidence.userInteractionScore += 0.5;
    }
  }

  /**
   * Record content quality from analysis
   */
  recordContentQuality(filePath: string, quality: number): void {
    const evidence = this.getOrCreateEvidence(filePath);
    evidence.contentQuality = quality;
    
    // Adjust score based on content quality
    if (quality > 80) {
      evidence.userInteractionScore += 2;
    } else if (quality > 50) {
      evidence.userInteractionScore += 1;
    }
  }

  /**
   * Get files that have evidence of being valuable despite not being in git
   */
  getValuableNonGitFiles(minScore: number = 5): FileUsageEvidence[] {
    return Array.from(this.usageData.values())
      .filter(e => !e.isGitTracked && e.userInteractionScore >= minScore)
      .sort((a, b) => b.userInteractionScore - a.userInteractionScore);
  }

  /**
   * Generate patterns for valuable non-git files
   */
  getRecommendedNonGitPatterns(minScore: number = 5): string[] {
    const valuableFiles = this.getValuableNonGitFiles(minScore);
    const patterns = new Set<string>();
    
    for (const file of valuableFiles) {
      const ext = path.extname(file.filePath);
      const dir = path.dirname(file.filePath);
      const fileName = path.basename(file.filePath);
      
      // Generate patterns based on file characteristics
      if (ext) {
        patterns.add(`**/*${ext}`); // Extension-based pattern
      }
      
      if (dir !== '.' && !dir.includes('node_modules')) {
        patterns.add(`${dir}/**`); // Directory-based pattern
      }
      
      // Specific file patterns for high-value files
      if (file.userInteractionScore > 10) {
        patterns.add(file.filePath); // Exact file pattern
      }
    }
    
    return Array.from(patterns);
  }

  /**
   * Generate comprehensive evidence report
   */
  generateEvidenceReport(): EvidenceReport {
    const allFiles = Array.from(this.usageData.values());
    const gitFiles = allFiles.filter(f => f.isGitTracked);
    const nonGitFiles = allFiles.filter(f => !f.isGitTracked);
    const valuableNonGit = this.getValuableNonGitFiles();
    
    const topInteraction = allFiles
      .sort((a, b) => b.userInteractionScore - a.userInteractionScore)
      .slice(0, 20);
    
    return {
      totalFiles: allFiles.length,
      gitTrackedFiles: gitFiles.length,
      nonGitFiles: nonGitFiles.length,
      valuableNonGitFiles: valuableNonGit,
      topInteractionFiles: topInteraction,
      recommendedPatterns: this.getRecommendedNonGitPatterns()
    };
  }

  /**
   * Log evidence summary
   */
  logEvidenceSummary(): void {
    const report = this.generateEvidenceReport();
    
    log('[Evidence] Usage Evidence Summary:');
    log(`  Total files tracked: ${report.totalFiles}`);
    log(`  Git-tracked files: ${report.gitTrackedFiles}`);
    log(`  Non-git files: ${report.nonGitFiles}`);
    log(`  Valuable non-git files: ${report.valuableNonGitFiles.length}`);
    
    if (report.valuableNonGitFiles.length > 0) {
      log('  Top valuable non-git files:');
      report.valuableNonGitFiles.slice(0, 5).forEach((file, i) => {
        log(`    ${i + 1}. ${file.filePath} (score: ${file.userInteractionScore}, changes: ${file.timesChanged})`);
      });
      
      log('  Recommended patterns:');
      report.recommendedPatterns.slice(0, 5).forEach(pattern => {
        log(`    - ${pattern}`);
      });
    }
  }

  private getOrCreateEvidence(filePath: string): FileUsageEvidence {
    if (!this.usageData.has(filePath)) {
      const now = new Date();
      this.usageData.set(filePath, {
        filePath,
        isGitTracked: false, // Will be updated when we know
        timesChanged: 0,
        timesSearched: 0,
        timesAccessed: 0,
        userInteractionScore: 0,
        firstSeen: now,
        lastAccessed: now,
        contentQuality: 0,
        fileSize: 0,
        extension: path.extname(filePath)
      });
    }
    return this.usageData.get(filePath)!;
  }

  private async loadExistingEvidence(): Promise<void> {
    try {
      const data = await fs.readFile(this.evidenceFile, 'utf-8');
      const evidenceArray = JSON.parse(data);
      
      for (const item of evidenceArray) {
        // Convert date strings back to Date objects
        item.firstSeen = new Date(item.firstSeen);
        item.lastAccessed = new Date(item.lastAccessed);
        this.usageData.set(item.filePath, item);
      }
      
      log(`[Evidence] Loaded ${this.usageData.size} existing evidence entries`);
    } catch (error) {
      // No existing evidence file, start fresh
      log('[Evidence] No existing evidence file, starting fresh');
    }
  }

  private async saveEvidence(): Promise<void> {
    try {
      // Ensure directory exists
      await fs.mkdir(path.dirname(this.evidenceFile), { recursive: true });
      
      // Convert Map to Array for JSON serialization
      const evidenceArray = Array.from(this.usageData.values());
      await fs.writeFile(this.evidenceFile, JSON.stringify(evidenceArray, null, 2));
      
      log(`[Evidence] Saved ${evidenceArray.length} evidence entries`);
    } catch (error) {
      log(`[Evidence] Failed to save evidence: ${error}`);
    }
  }

  private startPeriodicSave(): void {
    // Save evidence every 5 minutes
    this.saveInterval = setInterval(() => {
      this.saveEvidence();
    }, 5 * 60 * 1000);
  }

  async cleanup(): Promise<void> {
    if (this.saveInterval) {
      clearInterval(this.saveInterval);
    }
    await this.saveEvidence();
    log('[Evidence] Evidence collector cleaned up');
  }

  // Get statistics for monitoring
  getStats(): {
    totalFiles: number;
    gitFiles: number;
    nonGitFiles: number;
    averageScore: number;
    topScorer: string | null;
  } {
    const allFiles = Array.from(this.usageData.values());
    const gitFiles = allFiles.filter(f => f.isGitTracked).length;
    const nonGitFiles = allFiles.filter(f => !f.isGitTracked).length;
    const totalScore = allFiles.reduce((sum, f) => sum + f.userInteractionScore, 0);
    const averageScore = allFiles.length > 0 ? totalScore / allFiles.length : 0;
    
    const topScorer = allFiles.length > 0 
      ? allFiles.reduce((top, current) => 
          current.userInteractionScore > top.userInteractionScore ? current : top
        ).filePath
      : null;
    
    return {
      totalFiles: allFiles.length,
      gitFiles,
      nonGitFiles,
      averageScore,
      topScorer
    };
  }
}

export { FileUsageEvidence, EvidenceReport };