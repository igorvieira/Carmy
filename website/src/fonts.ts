// One font setup for the home page and the docs (Starlight's `head` takes the same
// attributes): Archivo for display, at its expanded widths like the logo lettering,
// Instrument Sans for text and JetBrains Mono for code.
const stylesheet =
  'https://fonts.googleapis.com/css2?family=Archivo:wdth,wght@62..125,400..900&family=Instrument+Sans:wght@400;500;600&family=JetBrains+Mono:wght@400;600&display=swap';

export const fontLinks: Record<string, string | boolean>[] = [
  { rel: 'preconnect', href: 'https://fonts.googleapis.com' },
  { rel: 'preconnect', href: 'https://fonts.gstatic.com', crossorigin: true },
  { rel: 'stylesheet', href: stylesheet },
];
