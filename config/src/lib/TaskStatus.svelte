<script lang="ts">
  import { CheckCircle, ExclamationTriangle, Icon, XMark } from "svelte-hero-icons";
  import { taskFeedback } from "./taskFeedback.svelte";
</script>

{#if taskFeedback.active}
  {@const task = taskFeedback.active}
  <div class="status {task.tone}" role={task.tone === "error" ? "alert" : "status"} aria-live="polite">
    {#if task.tone === "progress"}
      <span class="spinner" aria-hidden="true"></span>
    {:else if task.tone === "success"}
      <Icon src={CheckCircle} mini size="16px" />
    {:else}
      <Icon src={ExclamationTriangle} mini size="16px" />
    {/if}
    <span class="message">{task.message}</span>
    {#if taskFeedback.queuedCount}
      <span class="queued" title={`${taskFeedback.queuedCount} more message${taskFeedback.queuedCount === 1 ? "" : "s"}`}>+{taskFeedback.queuedCount}</span>
    {/if}
    {#if task.tone === "error" || task.tone === "warning"}
      <button onclick={() => taskFeedback.dismiss(task.id)} aria-label="Dismiss status">
        <Icon src={XMark} mini size="14px" />
      </button>
    {/if}
  </div>
{/if}

<style>
  .status { position: fixed; right: 18px; bottom: 18px; z-index: 50; display: flex; max-width: min(440px, calc(100vw - 36px)); min-height: 38px; padding: 8px 10px; align-items: center; gap: 7px; border: 1px solid var(--ui-border); border-radius: var(--ui-radius-md); background: var(--ui-surface-raised); color: var(--ui-muted); font-size: 12px; box-shadow: 0 10px 30px rgb(0 0 0 / .35); }
  .message { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .queued { flex: none; padding: 1px 5px; border-radius: 999px; background: rgb(0 0 0 / .2); font-size: 10px; font-weight: 700; }
  .success { color: var(--ui-success); border-color: var(--ui-success-border); background: var(--ui-success-bg); }
  .warning { color: var(--ui-warning); border-color: var(--ui-warning-border); background: var(--ui-warning-bg); }
  .error { color: var(--ui-danger); border-color: var(--ui-danger-border); background: var(--ui-danger-bg); }
  .spinner { width: 14px; height: 14px; flex: none; border: 2px solid var(--ui-accent); border-top-color: transparent; border-radius: 50%; animation: spin .7s linear infinite; }
  button { display: grid; width: 22px; height: 22px; padding: 0; place-items: center; border: 0; border-radius: 3px; background: transparent; color: currentColor; cursor: pointer; }
  button:hover { background: rgb(0 0 0 / .12); }
  @keyframes spin { to { transform: rotate(360deg); } }
</style>
