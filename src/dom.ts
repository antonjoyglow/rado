type Child = Node | string | null | undefined | false;

export function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  props: Partial<HTMLElementTagNameMap[K]> & { class?: string; data?: Record<string, string> } = {},
  ...children: Child[]
): HTMLElementTagNameMap[K] {
  const { class: className, data, ...rest } = props;
  const el = Object.assign(document.createElement(tag), rest);
  if (className) el.className = className;
  if (data) Object.assign(el.dataset, data);
  for (const child of children) {
    if (child) el.append(child);
  }
  return el;
}
