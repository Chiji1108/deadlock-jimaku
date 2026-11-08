<script lang="ts">
  // はじめかたのチェックリスト（DESIGN.md 13 章）。今の状態からチェックを付けるので、「完了」の操作はない。
  // 全部終わるか閉じたら、次からは出さない
  import CheckIcon from "@lucide/svelte/icons/check";
  import XIcon from "@lucide/svelte/icons/x";
  import { Button } from "$lib/components/ui/button";
  import * as Card from "$lib/components/ui/card";
  import { panel } from "../panel.svelte";
  import { CHROME, anyPaid, keyNames, providers } from "../../lib/engines";

  let { open }: { open: (tab: string) => void } = $props();

  const s = $derived(panel.settings!);
  /** 全部終わった画面は、この起動の間だけ出しておく（そのあと字幕を止めても崩さない） */
  let celebrating = $state(false);
  const steps = $derived([
    // 有料の工程がなければ鍵が要らないので、鍵の手順は鍵の要るモードを選んだ人にだけ出す
    ...(!anyPaid(s.engines)
      ? []
      : [
          {
            title: `このモードに要る ${keyNames(providers(s.engines))}を入れる`,
            done: celebrating || panel.hasAccess,
            tab: "keys",
            go: "入れる",
          },
        ]),
    {
      title: "OBS に字幕を追加する",
      done: celebrating || s.obsSeen,
      tab: "display",
      go: "字幕へ",
    },
    { title: "右上の「字幕を始める」を押す", done: celebrating || panel.running },
    { title: "話してみて、字幕が出るのを確かめる", done: celebrating || panel.subtitled },
  ]);
  const finished = $derived(steps.every((step) => step.done));

  $effect(() => {
    if (!finished || s.onboardingClosed) return;
    celebrating = true;
    void panel.update({ onboardingClosed: true });
  });

  function close() {
    celebrating = false;
    void panel.update({ onboardingClosed: true });
  }
</script>

{#if !s.onboardingClosed || celebrating}
  <Card.Root>
    <Card.Header>
      <Card.Title>はじめかた</Card.Title>
      {#if finished}
        <Card.Description>
          準備できました。アイテム説明やランクも、それぞれのタブから OBS に追加できます
        </Card.Description>
      {/if}
      <Card.Action>
        <Button variant="ghost" size="icon-sm" aria-label="閉じる" onclick={close}><XIcon /></Button
        >
      </Card.Action>
    </Card.Header>
    <Card.Content>
      <ol class="flex flex-col gap-2">
        {#each steps as step, i (step.title)}
          <li class="flex min-h-8 items-center gap-3 text-sm">
            <span
              class="flex size-6 shrink-0 items-center justify-center rounded-full text-xs font-semibold {step.done
                ? 'bg-primary text-primary-foreground'
                : 'border text-muted-foreground'}"
            >
              {#if step.done}<CheckIcon class="size-3.5" />{:else}{i + 1}{/if}
            </span>
            <span class={step.done ? "text-muted-foreground" : ""}>{step.title}</span>
            {#if step.tab && !step.done}
              <Button
                variant="link"
                size="sm"
                class="ml-auto h-auto p-0"
                onclick={() => open(step.tab!)}
              >
                {step.go}
              </Button>
              {#if i === 0 && anyPaid(s.engines)}
                <Button
                  variant="link"
                  size="sm"
                  class="h-auto p-0"
                  onclick={() => panel.update({ engines: CHROME })}
                >
                  無料の Chrome モードにする
                </Button>
              {/if}
            {/if}
          </li>
        {/each}
      </ol>
    </Card.Content>
  </Card.Root>
{/if}
