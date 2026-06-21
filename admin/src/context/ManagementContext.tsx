import { createContext, useContext } from 'react';
import type { ManagementInfo } from '@/api/client';

const ManagementContext = createContext<ManagementInfo | null>(null);

export function useManagementInfo() {
  return useContext(ManagementContext);
}

export function useGatewayApiEnabled(): boolean {
  return useManagementInfo()?.gateway_api_enabled === true;
}

export { ManagementContext };
