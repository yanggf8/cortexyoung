// Centralized stage constants to eliminate hard-coded values
export const STAGE_CONSTANTS = {
  TOTAL_STAGES: 3,
  STAGE_IDS: {
    INITIALIZATION: 'stage_1',
    CODE_INTELLIGENCE: 'stage_2', 
    SERVER_ACTIVATION: 'stage_3'
  },
  STAGE_NAMES: {
    stage_1: 'Initialization',
    stage_2: 'Code Intelligence Indexing',
    stage_3: 'Server Activation'
  }
} as const;

export type StageId = keyof typeof STAGE_CONSTANTS.STAGE_NAMES;