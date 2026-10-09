import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from 'react';
import { CheckCircle2, CircleAlert, X } from 'lucide-react';
import { Button } from '.';
const ToastContext = createContext<
  (text: string, tone?: 'success' | 'error') => void
>(() => undefined);
export const useToast = () => useContext(ToastContext);
export function ToastProvider({ children }: { children: ReactNode }) {
  const [items, setItems] = useState<
    { id: string; text: string; tone: 'success' | 'error' }[]
  >([]);
  const sequence = useRef(0);
  const timers = useRef(new Set<ReturnType<typeof setTimeout>>());
  useEffect(() => {
    const pending = timers.current;
    return () => {
      pending.forEach(clearTimeout);
      pending.clear();
    };
  }, []);
  const toast = useCallback(
    (text: string, tone: 'success' | 'error' = 'success') => {
      // crypto.randomUUID() throws outside secure contexts (plain http on a LAN).
      const id = String(++sequence.current);
      setItems((items) => [...items, { id, text, tone }]);
      const timer = setTimeout(() => {
        timers.current.delete(timer);
        setItems((items) => items.filter((item) => item.id !== id));
      }, 6000);
      timers.current.add(timer);
    },
    [],
  );
  return (
    <ToastContext.Provider value={toast}>
      {children}
      <div className="toasts" aria-live="polite">
        {items.map((item) => (
          <div className={`toast ${item.tone}`} key={item.id}>
            {item.tone === 'success' ? (
              <CheckCircle2 size={17} />
            ) : (
              <CircleAlert size={17} />
            )}
            <span>{item.text}</span>
            <Button
              variant="ghost"
              aria-label="Dismiss notification"
              onClick={() =>
                setItems((items) =>
                  items.filter((value) => value.id !== item.id),
                )
              }
            >
              <X size={15} />
            </Button>
          </div>
        ))}
      </div>
    </ToastContext.Provider>
  );
}
