<script lang="ts">
  // どのアプリが前にあっても字幕を始める・止めるキー（設定の `toggleHotkey`）。
  // 登録するのはデスクトップアプリで、押した結果は OS の通知で返る（DESIGN.md 13.6）
  import Help from "./Help.svelte";
  import * as Card from "$lib/components/ui/card";
  import { Button } from "$lib/components/ui/button";
  import { panel } from "../panel.svelte";

  /** OS に登録できる形（修飾キー + `KeyboardEvent.code`）にできるキー */
  const KEY = /^(Key[A-Z]|Digit\d|F\d{1,2})$/;
  const MODIFIERS = ["Control", "Shift", "Alt", "Meta"];
  const mac = navigator.platform.startsWith("Mac");

  const key = $derived(panel.settings?.toggleHotkey);
  /** 画面に出す名前（`Ctrl+Shift+KeyJ` → `Ctrl + Shift + J`） */
  const label = $derived(
    key
      ?.split("+")
      .map((part) => (part === "Super" ? (mac ? "⌘" : "Win") : part.replace(/^(Key|Digit)/, "")))
      .join(" + "),
  );
  let listening = $state(false);
  let hint = $state<string>();

  function listen() {
    listening = true;
    hint = undefined;
  }

  function onkeydown(event: KeyboardEvent) {
    if (!listening) return;
    event.preventDefault();
    if (event.key === "Escape") return void (listening = false);
    // 修飾キーだけの間は、次のキーを待つ
    if (MODIFIERS.includes(event.key)) return;
    const modifiers = [
      event.ctrlKey && "Ctrl",
      event.altKey && "Alt",
      event.shiftKey && "Shift",
      event.metaKey && "Super",
    ].filter((name) => name !== false);
    if (!KEY.test(event.code)) return void (hint = "英字・数字・F1〜F24 のキーを使ってください");
    // 文字のキーだけだと、ゲームやチャットで打つたびに切り替わる
    if (!modifiers.length && !event.code.startsWith("F"))
      return void (hint = "Ctrl や Shift と一緒に押してください（F1〜F24 は 1 つでも使えます）");
    listening = false;
    void panel.update({ toggleHotkey: [...modifiers, event.code].join("+") });
  }
</script>

<Card.Root>
  <Card.Header>
    <Card.Title class="flex items-center gap-1.5"
      >ホットキー<Help>
        ゲームをしたまま、キーで字幕を始めたり止めたりできます。押すと、始めたか止めたかを OS
        の通知で知らせます。デスクトップアプリが動いている間だけ効きます
      </Help></Card.Title
    >
  </Card.Header>
  <Card.Content class="flex flex-col gap-2">
    <div class="flex flex-wrap items-center gap-2">
      <Button
        variant="outline"
        class="min-w-40 justify-start tabular-nums"
        aria-label="ホットキーを決める"
        onclick={listen}
        {onkeydown}
        onblur={() => (listening = false)}
      >
        {#if listening}
          キーを押してください…
        {:else}
          {label ?? "キーを決める"}
        {/if}
      </Button>
      {#if key && !listening}
        <Button variant="ghost" size="sm" onclick={() => panel.update({ toggleHotkey: "" })}>
          外す
        </Button>
      {/if}
    </div>
    {#if listening}
      <p class="text-xs text-muted-foreground">{hint || "Esc でやめます"}</p>
    {/if}
  </Card.Content>
</Card.Root>
