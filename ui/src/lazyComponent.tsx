import { lazy, Suspense, type ComponentProps, type ComponentType, type ReactNode } from "react";

/**
 * A named export loaded on first render, so heavy views (terminals, the tree
 * canvas, settings, diffs) stay out of the entry chunk. It suspends inside its
 * own boundary, so the surrounding route never falls back to its pending state.
 */
export function lazyNamed<Module, Name extends keyof Module>(
  load: () => Promise<Module>,
  name: Name,
  fallback: ReactNode = null,
) {
  type Component = Module[Name] extends ComponentType<any> ? Module[Name] : never;
  const Lazy = lazy(async () => ({ default: (await load())[name] as ComponentType<any> }));
  function LazyNamed(props: ComponentProps<Component>) {
    return <Suspense fallback={fallback}><Lazy {...props} /></Suspense>;
  }
  LazyNamed.displayName = `Lazy(${String(name)})`;
  return LazyNamed;
}
