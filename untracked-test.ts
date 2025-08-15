// This is an untracked test file to verify dual-mode staging - MODIFIED
export function untrackedTestFunction(): string {
  return "This function should be staged and indexed even though it's not git-tracked - FINAL TEST";
}

export class UntrackedTestClass {
  private data: string;

  constructor(data: string) {
    this.data = data;
  }

  processData(): string {
    return `Processing: ${this.data}`;
  }

  async asyncOperation(): Promise<void> {
    // This should also be detectable by semantic search
    console.log("Performing async operation in untracked file");
  }
}

// Add some semantic patterns that should be detected
export interface UntrackedInterface {
  id: number;
  name: string;
  isActive: boolean;
}

export type UntrackedType = {
  config: UntrackedInterface;
  metadata: Record<string, any>;
};