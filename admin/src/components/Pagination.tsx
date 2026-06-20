type PaginationProps = {
  totalItems: number;
  pageSize: number;
  page: number;
  onPageChange: (next: number) => void;
};

function clamp(n: number, min: number, max: number) {
  return Math.max(min, Math.min(max, n));
}

export function Pagination({ totalItems, pageSize, page, onPageChange }: PaginationProps) {
  const totalPages = Math.max(1, Math.ceil(totalItems / pageSize));
  if (totalItems <= pageSize) return null;

  const safePage = clamp(page, 1, totalPages);
  const start = (safePage - 1) * pageSize + 1;
  const end = Math.min(totalItems, safePage * pageSize);

  return (
    <div className="flex flex-wrap items-center justify-between gap-3 border-t border-border pt-4 text-sm text-text-secondary">
      <span>
        Showing <span className="font-mono text-text">{start}</span>–<span className="font-mono text-text">{end}</span> of{' '}
        <span className="font-mono text-text">{totalItems}</span>
      </span>
      <div className="flex items-center gap-2">
        <button
          type="button"
          disabled={safePage <= 1}
          onClick={() => onPageChange(safePage - 1)}
          className="rounded-md border border-border px-3 py-1.5 hover:bg-hover disabled:opacity-40"
        >
          Prev
        </button>
        <span className="font-mono text-xs">
          {safePage} / {totalPages}
        </span>
        <button
          type="button"
          disabled={safePage >= totalPages}
          onClick={() => onPageChange(safePage + 1)}
          className="rounded-md border border-border px-3 py-1.5 hover:bg-hover disabled:opacity-40"
        >
          Next
        </button>
      </div>
    </div>
  );
}
