<script lang="ts">
  // 「モード」のタブ。モードを選ぶのはここだけ（「マイクと翻訳」は今のモードを出すだけ）。モードを選ぶと工程ごとの作り方がまとめて決まり、工程ごとにも変えられる（DESIGN.md 9.6）。
  // 押したらすぐ保存する
  import { Badge } from "$lib/components/ui/badge";
  import * as Card from "$lib/components/ui/card";
  import LocalGpu from "../parts/LocalGpu.svelte";
  import ModePicker from "../parts/ModePicker.svelte";
  import { panel } from "../panel.svelte";
  import {
    ITEMS,
    PROVIDER_NAMES,
    YEN_PER_USD,
    chromeFinals,
    llmTranslation,
    paidLive,
    providerOfOption,
    relayOf,
    unavailable,
    usesChrome,
  } from "../../lib/engines";
  import { translationOnly } from "../../lib/subtitle-lines";
  import type { Engines } from "../../generated/Engines";

  const s = $derived(panel.settings!);
  const t = $derived(s.engines);
  const items = Object.entries(ITEMS) as [keyof Engines, (typeof ITEMS)[keyof Engines]][];
  const choose = <K extends keyof Engines>(item: K, value: Engines[K]) =>
    panel.chooseMode({ ...t, [item]: value });

  /** その選択肢に要る鍵。会社を使わなければ undefined */
  function key<K extends keyof Engines>(item: K, value: Engines[K]) {
    const provider = providerOfOption(item, value);
    if (!provider) return undefined;
    const name = `${PROVIDER_NAMES[provider]} のキー`;
    return relayOf(provider) === "beta" ? `${name}（ベータ期間中は不要）` : name;
  }

  /** 選べない理由、または今の決まり方の説明 */
  function note(item: keyof Engines): string | undefined {
    if (item === "live" && translationOnly(s) && paidLive(t))
      return "翻訳だけを出しているので、途中の字幕は出ません";
    if (item === "live" && chromeFinals(t))
      return "確定の文字起こしが Chrome なら、途中の字幕も Chrome です";
    if (item === "live" && t.translation === "chromeTranslator")
      return "Chrome の翻訳を使っている間は、途中の字幕も Chrome です";
    if (item === "terms" && llmTranslation(t))
      return "翻訳が GPT なので、用語は GPT が翻訳と同時に判定します";
    if (item === "translation" && !usesChrome(t))
      return "Chrome の翻訳は、途中の字幕が Chrome のときだけ選べます";
    return undefined;
  }
</script>

<div class="flex flex-col gap-6">
  <Card.Root>
    <Card.Header>
      <Card.Title>モード</Card.Title>
      <Card.Description>選ぶと、下の工程がまとめて決まります</Card.Description>
    </Card.Header>
    <Card.Content><ModePicker /></Card.Content>
  </Card.Root>

  {#each items as [item, spec] (item)}
    <Card.Root>
      <Card.Header>
        <Card.Title>{spec.label}</Card.Title>
        <Card.Description
          >{spec.hint}{#if note(item)}。{note(item)}{/if}。{spec.scale}</Card.Description
        >
      </Card.Header>
      <Card.Content>
        <div
          class="flex flex-col divide-y rounded-lg border"
          role="radiogroup"
          aria-label={spec.label}
        >
          {#each spec.options as o (o.value)}
            {@const needs = key(item, o.value)}
            <button
              type="button"
              role="radio"
              aria-checked={t[item] === o.value}
              disabled={unavailable(t, item, o.value)}
              class="flex flex-col gap-1 p-3 text-left transition-colors first:rounded-t-lg last:rounded-b-lg hover:bg-muted disabled:opacity-50 disabled:hover:bg-transparent aria-checked:bg-muted"
              onclick={() => choose(item, o.value)}
            >
              <span class="flex flex-wrap items-center gap-2 text-sm">
                <span
                  class="size-3 shrink-0 rounded-full border border-muted-foreground"
                  class:bg-foreground={t[item] === o.value}
                  class:border-foreground={t[item] === o.value}
                ></span>
                <span class="font-medium">{o.label}</span>
                {#if needs}<Badge variant="outline">{needs}</Badge>{/if}
              </span>
              <!-- どの選択肢も、正確さ・速さ・料金の順に並べる（測っていないものは出さない） -->
              <span class="flex flex-wrap gap-x-3 pl-5 text-xs tabular-nums">
                {#if o.accuracy}<span>正確さ {o.accuracy}</span>{/if}
                {#if o.speed}<span>速さ {o.speed}</span>{/if}
                <span>{o.price}</span>
              </span>
              {#if o.note}<span class="pl-5 text-xs text-muted-foreground">{o.note}</span>{/if}
            </button>
          {/each}
        </div>
      </Card.Content>
    </Card.Root>
  {/each}
  <LocalGpu />
  <p class="text-sm text-muted-foreground">
    料金は、話し続けた 1 時間の目安です（1 ドル {YEN_PER_USD} 円で計算）。正確さと速さはデドロ字幕で測った値で、話し方や
    PC で変わります。切り替えると、字幕は始め直します。使った分の目安は「料金」タブに工程ごとに出ます
  </p>
</div>
