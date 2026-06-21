import { createContext, useContext } from 'react';

export type ApiMode = 'proxy' | 'ingress';

const ModeContext = createContext<ApiMode | undefined>(undefined);

export function useMode(): ApiMode | undefined {
  return useContext(ModeContext);
}

export { ModeContext };
