import { Moon, Sun } from 'lucide-react';
import { useTheme } from '@/context/ThemeContext';

export function ThemeToggle() {
  const { theme, setTheme } = useTheme();

  return (
    <div
      role="radiogroup"
      aria-label="Color theme"
      className="flex h-9 items-center gap-0.5 rounded-md border border-border bg-card p-0.5"
    >
      {(
        [
          { value: 'light' as const, label: 'Light', icon: Sun },
          { value: 'dark' as const, label: 'Dark', icon: Moon },
        ] as const
      ).map(({ value, label, icon: Icon }) => {
        const active = theme === value;
        return (
          <button
            key={value}
            type="button"
            role="radio"
            aria-checked={active}
            aria-label={label}
            title={label}
            onClick={() => setTheme(value)}
            className={`flex size-7 items-center justify-center rounded-[5px] transition-colors ${
              active ? 'bg-primary/15 text-primary' : 'text-muted-foreground hover:text-foreground'
            }`}
          >
            <Icon className="size-4" />
          </button>
        );
      })}
    </div>
  );
}
