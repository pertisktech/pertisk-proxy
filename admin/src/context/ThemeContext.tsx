import { createContext, useContext, useEffect, useState } from 'react';

const THEME_KEY = 'pertisk_theme';
type Theme = 'light' | 'dark';

type ThemeContextValue = {
  theme: Theme;
  isDark: boolean;
  toggleTheme: () => void;
  setTheme: (theme: Theme) => void;
};

const ThemeContext = createContext<ThemeContextValue | null>(null);

function getStoredTheme(): Theme {
  const stored = localStorage.getItem(THEME_KEY);
  return stored === 'light' ? 'light' : 'dark';
}

function applyTheme(theme: Theme) {
  const root = document.documentElement;
  root.classList.remove('light', 'dark');
  if (theme === 'dark') root.classList.add('dark');
  root.style.colorScheme = theme;
  // Hex only — Chrome rejects oklch/lab in <meta name="theme-color">.
  const bg = theme === 'light' ? '#f7f7f9' : '#1c1b22';
  root.style.backgroundColor = bg;
  const meta = document.getElementById('theme-color-meta');
  if (meta) meta.setAttribute('content', bg);
}

export function ThemeProvider({ children }: { children: React.ReactNode }) {
  const [theme, setTheme] = useState<Theme>(getStoredTheme);

  useEffect(() => {
    applyTheme(theme);
    localStorage.setItem(THEME_KEY, theme);
  }, [theme]);

  return (
    <ThemeContext.Provider
      value={{
        theme,
        isDark: theme === 'dark',
        toggleTheme: () => setTheme((t) => (t === 'dark' ? 'light' : 'dark')),
        setTheme,
      }}
    >
      {children}
    </ThemeContext.Provider>
  );
}

export function useTheme() {
  const ctx = useContext(ThemeContext);
  if (!ctx) throw new Error('useTheme outside provider');
  return ctx;
}
