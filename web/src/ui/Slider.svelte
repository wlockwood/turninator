<script lang="ts">
  let {
    label,
    value = $bindable(),
    min,
    max,
    step = 0.01,
    unit = '',
    digits = 2,
    disabled = false,
    onchange,
  }: {
    label: string;
    value: number;
    min: number;
    max: number;
    step?: number;
    unit?: string;
    digits?: number;
    disabled?: boolean;
    onchange?: (v: number) => void;
  } = $props();

  const id = `s-${Math.random().toString(36).slice(2, 9)}`;
</script>

<div class="slider" class:disabled>
  <label for={id}>{label}</label>
  <input
    {id}
    type="range"
    {min}
    {max}
    {step}
    {disabled}
    bind:value
    onchange={() => onchange?.(value)}
  />
  <span class="val">{value.toFixed(digits)}{unit}</span>
</div>

<style>
  .slider {
    display: grid;
    grid-template-columns: 6.8em 1fr 5em;
    align-items: center;
    gap: 8px;
    font-size: 0.85rem;
  }
  .disabled {
    opacity: 0.5;
  }
  label {
    color: var(--muted);
  }
  .val {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
</style>
