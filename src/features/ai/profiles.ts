import type { AiProfile, AiUsage } from '../../lib/types';

export interface CredentialStatus {
  status: 'notRequired' | 'missing' | 'saved' | 'session' | 'locked' | 'unavailable';
  present: boolean;
  message: string | null;
}
export interface AiProfileRecord {
  profile: AiProfile;
  requestUrl: string;
  credentialStatus: CredentialStatus;
}
export interface AiProfilePreview {
  profile: AiProfile;
  requestUrl: string;
  endpointIdentity: string;
  host: string;
}
export interface AiConnectionTest {
  profileId: string;
  host: string;
  model: string;
  requestUrl: string;
  text: string;
  usage: AiUsage | null;
}
export type CredentialStorage = 'keychain' | 'session';
