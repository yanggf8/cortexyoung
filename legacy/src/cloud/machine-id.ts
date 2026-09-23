import { createHash } from 'crypto';
import { hostname, userInfo } from 'os';

export function getMachineId(): string {
  return createHash('sha256')
    .update(hostname() + userInfo().username)
    .digest('hex')
    .slice(0, 12);
}
