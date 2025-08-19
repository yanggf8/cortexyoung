import { CodebaseIndexer } from './indexer';
import { SemanticSearcher } from './searcher';
import * as fs from 'fs';
import * as path from 'path';

export interface ProjectInfo {
  name: string;
  path: string;
  addedAt: number;
  lastAccessed: number;
  indexStatus: 'indexed' | 'indexing' | 'not_indexed' | 'error';
  fileCount?: number;
  indexSize?: number;
  error?: string;
}

export interface ProjectStats {
  fileCount: number;
  indexSize: number;
  lastIndexed: number;
  chunkCount: number;
}

export class ProjectManager {
  private projects: Map<string, ProjectInfo> = new Map();
  private currentProject: string | null = null;
  private indexers: Map<string, CodebaseIndexer> = new Map();
  private searchers: Map<string, SemanticSearcher> = new Map();
  private projectsFile: string;

  constructor(configDir: string = path.join(process.cwd(), '.cortex')) {
    this.projectsFile = path.join(configDir, 'projects.json');
    this.loadProjects();
  }

  private loadProjects(): void {
    try {
      if (fs.existsSync(this.projectsFile)) {
        const data = JSON.parse(fs.readFileSync(this.projectsFile, 'utf8'));
        this.projects = new Map(data.projects || []);
        this.currentProject = data.currentProject || null;
      }
    } catch (error) {
      console.warn('Failed to load projects configuration:', error);
    }
  }

  private saveProjects(): void {
    try {
      const dir = path.dirname(this.projectsFile);
      if (!fs.existsSync(dir)) {
        fs.mkdirSync(dir, { recursive: true });
      }

      const data = {
        projects: Array.from(this.projects.entries()),
        currentProject: this.currentProject,
        lastUpdated: Date.now()
      };

      fs.writeFileSync(this.projectsFile, JSON.stringify(data, null, 2));
    } catch (error) {
      console.warn('Failed to save projects configuration:', error);
    }
  }

  async addProject(projectPath: string, projectName?: string, startIndexing: boolean = true): Promise<ProjectInfo> {
    // Validate project path
    if (!fs.existsSync(projectPath)) {
      throw new Error(`Project path does not exist: ${projectPath}`);
    }

    if (!fs.statSync(projectPath).isDirectory()) {
      throw new Error(`Project path is not a directory: ${projectPath}`);
    }

    // Generate project name if not provided
    const name = projectName || path.basename(projectPath);
    const normalizedPath = path.resolve(projectPath);

    // Check if project already exists
    for (const [, project] of this.projects) {
      if (project.path === normalizedPath) {
        throw new Error(`Project already exists: ${project.name} (${normalizedPath})`);
      }
    }

    // Create project info
    const projectInfo: ProjectInfo = {
      name,
      path: normalizedPath,
      addedAt: Date.now(),
      lastAccessed: Date.now(),
      indexStatus: 'not_indexed'
    };

    // Add to projects
    this.projects.set(name, projectInfo);
    this.saveProjects();

    // Start indexing if requested
    if (startIndexing) {
      try {
        await this.indexProject(name);
      } catch (error) {
        projectInfo.indexStatus = 'error';
        projectInfo.error = error instanceof Error ? error.message : 'Unknown error';
        this.saveProjects();
      }
    }

    return projectInfo;
  }

  async switchProject(projectPath: string, projectName?: string, autoIndex: boolean = true): Promise<ProjectInfo> {
    const normalizedPath = path.resolve(projectPath);
    
    // Check if project exists in our registry
    let projectInfo: ProjectInfo | undefined;
    let projectKey: string | undefined;
    
    for (const [key, project] of this.projects) {
      if (project.path === normalizedPath) {
        projectInfo = project;
        projectKey = key;
        break;
      }
    }

    // If project not found, add it first
    if (!projectInfo) {
      projectInfo = await this.addProject(projectPath, projectName, autoIndex);
      projectKey = projectInfo.name;
    }

    // Update current project
    this.currentProject = projectKey!;
    projectInfo.lastAccessed = Date.now();
    this.saveProjects();

    // Index if needed and requested
    if (autoIndex && projectInfo.indexStatus !== 'indexed') {
      try {
        await this.indexProject(projectKey!);
      } catch (error) {
        projectInfo.indexStatus = 'error';
        projectInfo.error = error instanceof Error ? error.message : 'Unknown error';
        this.saveProjects();
      }
    }

    return projectInfo;
  }

  getCurrentProject(): ProjectInfo | null {
    if (!this.currentProject) return null;
    return this.projects.get(this.currentProject) || null;
  }

  async listProjects(includeStats: boolean = true): Promise<ProjectInfo[]> {
    const projects = Array.from(this.projects.values());
    
    if (includeStats) {
      // Add basic stats for each project
      for (const project of projects) {
        if (project.indexStatus === 'indexed') {
          try {
            const stats = await this.getProjectStats(project.name);
            project.fileCount = stats?.fileCount;
            project.indexSize = stats?.indexSize;
          } catch (error) {
            // Stats not available
          }
        }
      }
    }

    return projects.sort((a, b) => b.lastAccessed - a.lastAccessed);
  }

  async indexProject(projectName: string): Promise<void> {
    const project = this.projects.get(projectName);
    if (!project) {
      throw new Error(`Project not found: ${projectName}`);
    }

    if (!fs.existsSync(project.path)) {
      throw new Error(`Project path no longer exists: ${project.path}`);
    }

    project.indexStatus = 'indexing';
    this.saveProjects();

    try {
      // Create indexer for this project
      const indexer = new CodebaseIndexer(project.path);
      await indexer.indexRepository({
        repository_path: project.path,
        mode: 'incremental'
      });
      
      // Store indexer for future use
      this.indexers.set(projectName, indexer);
      
      // Get searcher from indexer (it's created internally)
      const searcher = (indexer as any).searcher;
      this.searchers.set(projectName, searcher);

      project.indexStatus = 'indexed';
      project.error = undefined;
      this.saveProjects();

    } catch (error) {
      project.indexStatus = 'error';
      project.error = error instanceof Error ? error.message : 'Unknown error';
      this.saveProjects();
      throw error;
    }
  }

  async getProjectStats(projectName: string): Promise<ProjectStats | null> {
    const indexer = this.indexers.get(projectName);
    if (!indexer) return null;

    try {
      const stats = await indexer.getIndexStats();
      return {
        fileCount: stats.total_chunks, // Approximate - each file typically has multiple chunks
        indexSize: stats.total_chunks,
        lastIndexed: stats.last_indexed ? new Date(stats.last_indexed).getTime() : Date.now(),
        chunkCount: stats.total_chunks
      };
    } catch (error) {
      return null;
    }
  }

  getCurrentIndexer(): CodebaseIndexer | null {
    if (!this.currentProject) return null;
    return this.indexers.get(this.currentProject) || null;
  }

  getCurrentSearcher(): SemanticSearcher | null {
    if (!this.currentProject) return null;
    return this.searchers.get(this.currentProject) || null;
  }

  async ensureCurrentProjectIndexed(): Promise<void> {
    const current = this.getCurrentProject();
    if (!current) {
      throw new Error('No current project set');
    }

    if (current.indexStatus !== 'indexed') {
      await this.indexProject(current.name);
    }
  }

  // Initialize with the current working directory as the first project
  async initializeWithCurrentDirectory(): Promise<void> {
    const cwd = process.cwd();
    const cwdName = path.basename(cwd);
    
    // Check if current directory is already a project
    let hasCurrentProject = false;
    for (const [, project] of this.projects) {
      if (project.path === cwd) {
        this.currentProject = project.name;
        hasCurrentProject = true;
        break;
      }
    }

    // If no current project, add the current directory
    if (!hasCurrentProject) {
      await this.addProject(cwd, cwdName, true);
      this.currentProject = cwdName;
      this.saveProjects();
    }
  }
}