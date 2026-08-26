export class CortError extends Error {
  constructor(code, detail = null) {
    super(`${code}: ${JSON.stringify(detail)}`);
    this.name = 'CortError';
    this.code = code;
    this.detail = detail;
  }
  toJSON() { return { error: this.code, detail: this.detail }; }
}
