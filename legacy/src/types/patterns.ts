export interface ImplementationPatterns {
  authentication: {
    userProperty: string | 'Unknown';      // "req.user" | "req.context.user" | "req.auth" | "Unknown"
    tokenLocation: string | 'Unknown';     // "httpOnly cookie" | "authorization header" | "Unknown"
    errorResponse: {
      format: string | 'Unknown';          // "{error: string}" | "{message: string}" | "Unknown" 
      statusCode: number | 0;              // 401 | 403 | 0 (unknown)
    };
    middlewarePattern: string | 'Unknown'; // "app.use(auth)" | "@authenticated" | "Unknown"
    confidence: number;
    evidence: string[];
  };
  apiResponses: {
    successFormat: string | 'Unknown';     // "{data: any}" | "{result: any}" | "bare object" | "Unknown"
    errorFormat: string | 'Unknown';       // "{error: string}" | "{message: string}" | "Unknown"
    statusCodeUsage: string | 'Unknown';   // "explicit codes" | "default 200/500" | "Unknown"
    wrapperPattern: string | 'Unknown';    // "always wrapped" | "conditional" | "Unknown"
    confidence: number;
    evidence: string[];
  };
  errorHandling: {
    catchPattern: string | 'Unknown';      // "global middleware" | "try/catch blocks" | "Result types" | "Unknown"
    errorStructure: string | 'Unknown';    // "{error: string, code?: string}" | "{message: string}" | "Unknown"
    loggingIntegration: string | 'Unknown'; // "integrated" | "separate" | "Unknown"
    propagationStyle: string | 'Unknown';  // "throw exceptions" | "return errors" | "Unknown"
    confidence: number;
    evidence: string[];
  };
}

export interface ProjectContextInfo {
  projectType: string;
  language: string;
  framework: string;
  packageManager: string;
  directories: Array<{path: string, purpose: string}>;
  dependencies: string[];
  scripts: {dev?: string, build?: string, test?: string};
  implementationPatterns: ImplementationPatterns;
  lastUpdated: string;
}

export interface PatternDetectionSignal {
  weight: number;
  detected: boolean;
  evidence?: string;
}

export interface ConfidenceCalculation {
  signals: PatternDetectionSignal[];
  totalConfidence: number;
  uncertaintyWarning?: string;
}