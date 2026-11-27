import { useEffect, useRef, useState } from 'react';

function isPromiseLike<T>(value: unknown): value is PromiseLike<T> {
  return typeof value === 'object' && value !== null && 'then' in value && typeof (value as PromiseLike<T>).then === 'function';
}

export function useActionDone(resetKey?: string) {
  const [doneKey, setDoneKey] = useState<string | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const running = useRef<string | null>(null);
  const generation = useRef(0);
  const lastReset = useRef(resetKey);

  useEffect(() => {
    if (lastReset.current !== resetKey) {
      lastReset.current = resetKey;
      ++generation.current;
      running.current = null;
      clearTimeout(timer.current);
      setDoneKey(null);
    }
  }, [resetKey]);

  useEffect(() => () => {
    ++generation.current;
    clearTimeout(timer.current);
  }, []);

  function finish(key: string, current: number, result: boolean | void, animate: boolean) {
    if (current !== generation.current) return;
    if (animate && result !== false) {
      setDoneKey(key);
      clearTimeout(timer.current);
      timer.current = setTimeout(() => {
        setDoneKey((value) => (value === key ? null : value));
      }, 1000);
    } else if (!animate && doneKey === key) {
      clearTimeout(timer.current);
      setDoneKey(null);
    }
  }

  function run(
    key: string,
    action: () => boolean | void | Promise<boolean | void>,
    options?: { lockWhileDone?: boolean; animate?: boolean },
  ) {
    const lockWhileDone = options?.lockWhileDone ?? false;
    const animate = options?.animate ?? true;
    if (lockWhileDone && (running.current === key || doneKey === key)) return;
    if (lockWhileDone) running.current = key;
    const current = generation.current;
    const outcome = action();
    if (!isPromiseLike<boolean | void>(outcome)) {
      if (lockWhileDone && running.current === key) running.current = null;
      finish(key, current, outcome, animate);
      return;
    }
    void Promise.resolve(outcome).then(
      (result) => { finish(key, current, result, animate); },
      () => {},
    ).finally(() => {
      if (lockWhileDone && current === generation.current && running.current === key) running.current = null;
    });
  }

  return { doneKey, run };
}
