import { useEffect, useState } from 'react';

export function usePageSize(rowHeight = 44, overhead = 260, min = 5): number {
  const compute = () => Math.max(min, Math.floor((window.innerHeight - overhead) / rowHeight));
  const [pageSize, setPageSize] = useState(compute);

  useEffect(() => {
    const onResize = () => setPageSize(compute());
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  }, [rowHeight, overhead, min]);

  return pageSize;
}
