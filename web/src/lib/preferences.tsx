import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from 'react';
export type Preferences = {
  density: 'comfortable' | 'compact';
  timezone: 'local' | 'utc';
  editorFontSize: number;
};
const defaults: Preferences = {
  density: 'comfortable',
  timezone: 'local',
  editorFontSize: 13,
};
export function readPreferences(): Preferences {
  try {
    const stored: unknown = JSON.parse(
      localStorage.getItem('vda.preferences') ?? 'null',
    );
    if (!stored || typeof stored !== 'object') return defaults;
    const value = stored as Partial<Preferences>;
    return {
      density: value.density === 'compact' ? 'compact' : 'comfortable',
      timezone: value.timezone === 'utc' ? 'utc' : 'local',
      editorFontSize:
        value.editorFontSize === 18
          ? 20
          : typeof value.editorFontSize === 'number' &&
              [12, 13, 14, 16, 20].includes(value.editorFontSize)
            ? value.editorFontSize
            : 13,
    };
  } catch {
    return defaults;
  }
}
const PreferencesContext = createContext<{
  preferences: Preferences;
  setPreferences: (value: Preferences) => void;
}>({ preferences: defaults, setPreferences: () => undefined });
export const usePreferences = () => useContext(PreferencesContext);
export function PreferencesProvider({ children }: { children: ReactNode }) {
  const [preferences, setPreferences] = useState(readPreferences);
  useEffect(() => {
    // Settings apply and persist as they change; storage can be unavailable
    // (private windows, blocked site data), so failure keeps the live value.
    try {
      localStorage.setItem('vda.preferences', JSON.stringify(preferences));
    } catch {
      /* not persisted in this browser */
    }
    document.documentElement.dataset.density = preferences.density;
    document.documentElement.style.setProperty(
      '--editor-font-size',
      `${preferences.editorFontSize}px`,
    );
  }, [preferences]);
  const value = useMemo(() => ({ preferences, setPreferences }), [preferences]);
  return (
    <PreferencesContext.Provider value={value}>
      {children}
    </PreferencesContext.Provider>
  );
}
