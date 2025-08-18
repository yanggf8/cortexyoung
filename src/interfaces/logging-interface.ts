// Unified logging interface to eliminate double logging
import { STAGE_CONSTANTS } from '../constants/stage-constants';

export interface UnifiedLogger {
  // Stage logging
  startStage(stageNumber: number, stageName: string): void;
  completeStage(stageNumber: number, stageName: string, duration: string): void;
  failStage(stageNumber: number, stageName: string, error: string, duration: string): void;
  
  // Substep logging
  startSubstep(substepId: string, substepName: string, details?: string): void;
  completeSubstep(substepId: string, substepName: string, details?: string): void;
  failSubstep(substepId: string, substepName: string, error: string): void;
  
  // General logging
  info(message: string): void;
  warn(message: string): void;
  error(message: string): void;
}

export class CompositeLogger implements UnifiedLogger {
  private useEnhancedLogging: boolean;

  constructor() {
    // Use consistent environment variable naming
    this.useEnhancedLogging = process.env.CORTEX_ENABLE_NEW_LOGGING === 'true';
  }

  startStage(stageNumber: number, stageName: string): void {
    if (this.useEnhancedLogging) {
      const { conditionalLogger } = require('../utils/console-logger');
      conditionalLogger.stage.start(stageNumber, STAGE_CONSTANTS.TOTAL_STAGES, stageName);
    } else {
      const { timestampedLog } = require('../logging-utils');
      timestampedLog('==========================================');
      timestampedLog(`🚀 STAGE ${stageNumber}/${STAGE_CONSTANTS.TOTAL_STAGES}: ${stageName.toUpperCase()}`);
      timestampedLog('==========================================');
    }
  }

  completeStage(stageNumber: number, stageName: string, duration: string): void {
    if (this.useEnhancedLogging) {
      const { conditionalLogger } = require('../utils/console-logger');
      conditionalLogger.stage.complete(stageNumber, STAGE_CONSTANTS.TOTAL_STAGES, stageName, { duration });
    } else {
      const { timestampedLog } = require('../logging-utils');
      timestampedLog('==========================================');
      timestampedLog(`✅ STAGE ${stageNumber}/${STAGE_CONSTANTS.TOTAL_STAGES} COMPLETED: ${stageName.toUpperCase()}`);
      timestampedLog(`   Duration: ${duration}`);
      timestampedLog('==========================================');
    }
  }

  failStage(stageNumber: number, stageName: string, error: string, duration: string): void {
    if (this.useEnhancedLogging) {
      const { conditionalLogger } = require('../utils/console-logger');
      conditionalLogger.stage.fail(stageNumber, STAGE_CONSTANTS.TOTAL_STAGES, stageName, error);
    } else {
      const { timestampedError } = require('../logging-utils');
      timestampedError('==========================================');
      timestampedError(`❌ STAGE ${stageNumber}/${STAGE_CONSTANTS.TOTAL_STAGES} FAILED: ${stageName.toUpperCase()}`);
      timestampedError(`   Error: ${error}`);
      timestampedError(`   Duration: ${duration}`);
      timestampedError('==========================================');
    }
  }

  startSubstep(substepId: string, substepName: string, details?: string): void {
    if (this.useEnhancedLogging) {
      const { conditionalLogger } = require('../utils/console-logger');
      conditionalLogger.step.start(substepId, substepName, details);
    } else {
      const { timestampedLog } = require('../logging-utils');
      const detailsText = details ? ` - ${details}` : '';
      timestampedLog(`   ⏳ ${substepId}: ${substepName}${detailsText}`);
    }
  }

  completeSubstep(substepId: string, substepName: string, details?: string): void {
    if (this.useEnhancedLogging) {
      const { conditionalLogger } = require('../utils/console-logger');
      conditionalLogger.step.complete(substepId, substepName, details);
    } else {
      const { timestampedLog } = require('../logging-utils');
      const detailsText = details ? ` - ${details}` : '';
      timestampedLog(`   ✅ ${substepId}: ${substepName}${detailsText}`);
    }
  }

  failSubstep(substepId: string, substepName: string, error: string): void {
    if (this.useEnhancedLogging) {
      const { conditionalLogger } = require('../utils/console-logger');
      conditionalLogger.step.fail(substepId, substepName, error);
    } else {
      const { timestampedError } = require('../logging-utils');
      timestampedError(`   ❌ ${substepId}: ${substepName} - ${error}`);
    }
  }

  info(message: string): void {
    if (this.useEnhancedLogging) {
      const { conditionalLogger } = require('../utils/console-logger');
      conditionalLogger.logger.info(message);
    } else {
      const { timestampedLog } = require('../logging-utils');
      timestampedLog(message);
    }
  }

  warn(message: string): void {
    if (this.useEnhancedLogging) {
      const { conditionalLogger } = require('../utils/console-logger');
      conditionalLogger.logger.warn(message);
    } else {
      const { timestampedWarn } = require('../logging-utils');
      timestampedWarn(message);
    }
  }

  error(message: string): void {
    if (this.useEnhancedLogging) {
      const { conditionalLogger } = require('../utils/console-logger');
      conditionalLogger.logger.error(message);
    } else {
      const { timestampedError } = require('../logging-utils');
      timestampedError(message);
    }
  }
}