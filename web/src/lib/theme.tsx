import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from 'react';
type Theme = 'light' | 'dark' | 'system';
function storedTheme(): Theme {
  try {
    const value = localStorage.getItem('vda.theme');
    return value === 'light' || value === 'dark' ? value : 'system';
  } catch {
    return 'system';
  }
}
const ThemeContext = createContext<{
  theme: Theme;
  setTheme: (theme: Theme) => void;
  resolved: 'light' | 'dark';
}>({ theme: 'system', setTheme: () => undefined, resolved: 'light' });
export const useTheme = () => useContext(ThemeContext);
export function ThemeProvider({ children }: { children: ReactNode }) {
  const [theme, setTheme] = useState<Theme>(storedTheme),
    [system, setSystem] = useState<'light' | 'dark'>(() =>
      typeof matchMedia === 'function' &&
      matchMedia('(prefers-color-scheme: dark)').matches
        ? 'dark'
        : 'light',
    );
  const resolved = theme === 'system' ? system : theme;
  useEffect(() => {
    if (typeof matchMedia !== 'function') return;
    const media = matchMedia('(prefers-color-scheme: dark)');
    const update = () => setSystem(media.matches ? 'dark' : 'light');
    media.addEventListener('change', update);
    return () => media.removeEventListener('change', update);
  }, []);
  useEffect(() => {
    document.documentElement.dataset.theme = resolved;
    try {
      localStorage.setItem('vda.theme', theme);
    } catch {
      /* The selected theme still applies when storage is unavailable. */
    }
  }, [theme, resolved]);
  const value = useMemo(
    () => ({ theme, setTheme, resolved }),
    [theme, resolved],
  );
  return (
    <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>
  );
}
