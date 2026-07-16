<script lang="ts">
  import { CheckCircle, ExclamationTriangle, Icon, XMark } from "svelte-hero-icons";
  import { taskFeedback } from "./taskFeedback.svelte.js";
</script>

{#if taskFeedback.active}
  {@const task = taskFeedback.active}
  <div class="status {task.tone}" role={task.tone === "error" ? "alert" : "status"} aria-live="polite" title={task.message}>
    {#if task.tone === "progress"}<span class="spinner"></span>
    {:else if task.tone === "success"}<Icon src={CheckCircle} mini size="14px" />
    {:else}<Icon src={ExclamationTriangle} mini size="14px" />{/if}
    <span class="message">{task.message}{#if task.current !== null && task.total !== null} {task.current}/{task.total}{/if}</span>
    {#if taskFeedback.queuedCount}<span class="queued" title={`${taskFeedback.queuedCount} more status message${taskFeedback.queuedCount === 1 ? "" : "s"}`}>+{taskFeedback.queuedCount}</span>{/if}
    {#if task.tone === "error" || task.tone === "warning"}<button onclick={() => taskFeedback.dismiss(task.id)} aria-label="Dismiss status"><Icon src={XMark} mini size="13px" /></button>{/if}
  </div>
{/if}

<style>
  .status { display: flex; width: max-content; max-width: min(460px, 45vw); min-width: 180px; height: 26px; padding: 0 7px; align-items: center; gap: 5px; border: 1px solid var(--ui-border); border-radius: var(--ui-radius-sm); background: var(--ui-bg); color: var(--ui-muted); font-family: var(--ui-font-mono); font-size: 11px; }
  /* Progress appears only once an operation has taken a moment — fast tasks never flash a badge. */
  .progress { animation: status-appear 120ms ease 250ms backwards; }
  @keyframes status-appear { from { opacity: 0; } }
  .message { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .queued { flex: none; padding: 1px 4px; border-radius: 999px; background: rgb(0 0 0 / .08); font-size: 9px; font-weight: 700; }
  .warning { color: var(--ui-warning); border-color: var(--ui-warning-border); background: var(--ui-warning-bg); }
  .error { color: var(--ui-danger); border-color: var(--ui-danger-border); background: var(--ui-danger-bg); }
  .spinner { width: 12px; height: 12px; flex: none; border: 2px solid var(--ui-accent); border-top-color: transparent; border-radius: 50%; animation: spin .7s linear infinite; }
  button { display: grid; width: 20px; height: 20px; padding: 0; place-items: center; border: 0; border-radius: 3px; background: transparent; color: currentColor; cursor: pointer; }
  button:hover { background: rgb(0 0 0 / .07); } button:focus-visible { outline: 2px solid var(--ui-focus); }
  @media (max-width: 760px) { .status { min-width: 0; max-width: 150px; } }
  @media (max-width: 560px) { .status { width: auto; } .message { display: none; } }
  @keyframes spin { to { transform: rotate(360deg); } }
</style>
