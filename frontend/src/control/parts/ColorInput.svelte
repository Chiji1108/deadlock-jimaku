<script lang="ts">
  import { Input } from "$lib/components/ui/input";

  // OS の色選びと #rrggbb の入力。選んでいる間も字幕に反映するが、送りすぎないよう間隔を空ける
  const HEX = /^#[0-9a-fA-F]{6}$/;
  const INTERVAL_MS = 150;

  let { id, value, onchange }: { id: string; value: string; onchange: (hex: string) => unknown } =
    $props();

  /** 打っている途中や、送って保存されるまでの値 */
  let edited = $state<string | undefined>();
  const text = $derived(edited ?? value);
  let timer: ReturnType<typeof setTimeout> | undefined;

  async function send() {
    clearTimeout(timer);
    timer = undefined;
    const sent = edited;
    if (!sent || !HEX.test(sent)) return;
    await onchange(sent.toLowerCase());
    if (edited === sent) edited = undefined;
  }

  function input(next: string) {
    edited = next;
    if (HEX.test(next)) timer ??= setTimeout(send, INTERVAL_MS);
  }

  /** 欄を離れたら、待っている分を送る。形式が合わない入力は捨てて今の色に戻す */
  function leave() {
    if (edited && !HEX.test(edited)) edited = undefined;
    else if (timer) void send();
  }
</script>

<div class="flex items-center gap-2">
  <input
    type="color"
    class="size-9 cursor-pointer rounded-md border-0 bg-transparent p-0"
    aria-label="色を選ぶ"
    value={HEX.test(text) ? text : value}
    oninput={(e) => input(e.currentTarget.value)}
  />
  <Input
    {id}
    class="w-28 font-mono"
    value={text}
    oninput={(e) => input(e.currentTarget.value)}
    onblur={leave}
  />
</div>
