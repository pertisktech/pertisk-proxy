import { useEffect, useRef } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';

/**
 * When the URL has `?{param}=1` (or any truthy value), call `onOpen` once and
 * strip the query param so a refresh does not reopen the modal.
 */
export function useOpenOnQuery(param: string, onOpen: () => void) {
  const location = useLocation();
  const navigate = useNavigate();
  const onOpenRef = useRef(onOpen);
  onOpenRef.current = onOpen;

  useEffect(() => {
    const params = new URLSearchParams(location.search);
    const raw = params.get(param);
    if (!raw || raw === '0' || raw.toLowerCase() === 'false') return;

    onOpenRef.current();
    params.delete(param);
    const search = params.toString();
    navigate(
      { pathname: location.pathname, search: search ? `?${search}` : '', hash: location.hash },
      { replace: true },
    );
  }, [location.pathname, location.search, location.hash, param, navigate]);
}
