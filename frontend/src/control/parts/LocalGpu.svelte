<script lang="ts">
  // PC の中の AI（Haas-ja、Whisper、Gemma）に使わせる GPU を選ぶ。GPU が 2 つ以上ある PC でだけ出す
  // （内蔵の GPU と、あとから付けた GPU がある PC。DESIGN.md 9.11）
  import * as Card from "$lib/components/ui/card";
  import * as Select from "$lib/components/ui/select";
  import type { Gpus } from "../../generated/Gpus";
  import { api } from "../../lib/api";
  import { panel } from "../panel.svelte";

  const AUTO = "auto";
  const s = $derived(panel.settings!);
  let listed = $state<Gpus | null>(null);
  const load = async () => (listed = await api.gpus().catch(() => null));
  void load();

  const auto = $derived(`自動（${listed?.auto ?? "いちばん高性能な GPU"}）`);
  const label = (gpu: Gpus["gpus"][number]) =>
    `${gpu.name}（${gpu.integrated ? "内蔵・" : ""}${gpu.gb} GB）`;
</script>

{#if listed && listed.gpus.length > 1}
  <Card.Root>
    <Card.Header>
      <Card.Title>PC の中の AI に使う GPU</Card.Title>
      <Card.Description>
        Haas-ja・Whisper・Gemma を動かす GPU です。自動は、内蔵でない GPU
        を選びます。ゲームと分けたいときは、内蔵の GPU を選べます（遅くなります）
      </Card.Description>
    </Card.Header>
    <Card.Content>
      <Select.Root
        type="single"
        value={s.localGpu ?? AUTO}
        onValueChange={(v) => panel.update({ localGpu: v === AUTO ? "" : v })}
        onOpenChange={(open) => open && void load()}
      >
        <Select.Trigger class="w-full">{s.localGpu ?? auto}</Select.Trigger>
        <Select.Content>
          <Select.Item value={AUTO}>{auto}</Select.Item>
          {#each listed.gpus as gpu (gpu.id)}
            <Select.Item value={gpu.name}>{label(gpu)}</Select.Item>
          {/each}
        </Select.Content>
      </Select.Root>
    </Card.Content>
  </Card.Root>
{/if}
