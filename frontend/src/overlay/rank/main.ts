import { mount } from "svelte";
import "../../lib/lang.css";
import { fitSource, RANK_SIZE } from "../../lib/overlay-size";
import Rank from "./Rank.svelte";

// 大きさは --scale で描く（zoom だとポイントの数字のアニメーションの位置がずれる）
fitSource(RANK_SIZE, (scale) =>
  document.documentElement.style.setProperty("--scale", String(scale)),
);

mount(Rank, { target: document.getElementById("app")! });
