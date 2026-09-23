import { readFileSync, existsSync } from 'fs';
import { join, basename } from 'path';
import { execFileSync } from 'child_process';

export interface ProjectIdentity {
  id: string;
  displayName: string;
}

export class ProjectIdentifier {
  static resolve(projectRoot: string): ProjectIdentity {
    // Strategy 1: git remote origin URL
    const gitId = ProjectIdentifier.fromGitRemote(projectRoot);
    if (gitId) return gitId;

    // Strategy 2: package.json name
    const pkgId = ProjectIdentifier.fromPackageJson(projectRoot);
    if (pkgId) return pkgId;

    // Strategy 3: directory basename
    const name = basename(projectRoot);
    return { id: `dir:${name}`, displayName: name };
  }

  private static fromGitRemote(projectRoot: string): ProjectIdentity | null {
    try {
      const url = execFileSync('git', ['remote', 'get-url', 'origin'], {
        cwd: projectRoot,
        encoding: 'utf-8',
        stdio: ['pipe', 'pipe', 'pipe'],
      }).trim();

      if (!url) return null;

      // Normalize: remove .git suffix, convert SSH to comparable form
      const normalized = url.replace(/\.git$/, '').replace(/^git@([^:]+):/, 'https://$1/');
      const displayName = normalized.split('/').pop() || basename(projectRoot);

      return { id: `git:${normalized}`, displayName };
    } catch {
      return null;
    }
  }

  private static fromPackageJson(projectRoot: string): ProjectIdentity | null {
    const pkgPath = join(projectRoot, 'package.json');
    if (!existsSync(pkgPath)) return null;

    try {
      const pkg = JSON.parse(readFileSync(pkgPath, 'utf-8'));
      if (pkg.name) {
        return { id: `pkg:${pkg.name}`, displayName: pkg.name };
      }
      return null;
    } catch {
      return null;
    }
  }
}
