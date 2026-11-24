// shadcn-svelte の部品が使う（アプリの frontend/src/lib/utils.ts と同じ）
export { cn } from "cn";

export type WithElementRef<T, U extends HTMLElement = HTMLElement> = T & { ref?: U | null };
