/** Keeps decoded WebView image strings within a predictable memory budget. */
export class DataUrlCache {
  private values = new Map<string, string>();
  private bytes = 0;

  constructor(private readonly maxBytes: number) {}

  get(key: string): string | undefined {
    const value = this.values.get(key);
    if (value !== undefined) {
      this.values.delete(key);
      this.values.set(key, value);
    }
    return value;
  }

  set(key: string, value: string): void {
    this.delete(key);
    // JavaScript strings may use two bytes per character. Use that upper bound.
    const cost = value.length * 2;
    if (cost > this.maxBytes) return;
    while (this.bytes + cost > this.maxBytes) {
      const oldest = this.values.keys().next().value;
      if (oldest === undefined) break;
      this.delete(oldest);
    }
    this.values.set(key, value);
    this.bytes += cost;
  }

  delete(key: string): void {
    const value = this.values.get(key);
    if (value === undefined) return;
    this.values.delete(key);
    this.bytes -= value.length * 2;
  }
}
