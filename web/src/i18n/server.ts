import { currentLanguage } from './index.ts';
import { serverTranslator, type ServerTranslator } from './server-match.ts';
import { exact, patterns } from './server.pl.ts';

let polish: ServerTranslator | undefined;

/**
 * What the server says, in the language of the interface. The server writes English: error
 * messages, notification texts, the status lines of agent runs, the lines of import reports.
 * In Polish a text the dictionary (`server.pl.ts`) knows comes back in Polish, exactly or by
 * pattern; anything else comes back as it is. So it is safe on a text that may be the server's
 * or a person's, but it is not meant for what people write (summaries, comments, answers).
 */
export function translateServer(text: string): string {
  if (currentLanguage() !== 'pl' || !text) {
    return text;
  }

  polish ??= serverTranslator(exact, patterns);

  return polish.translate(text);
}
