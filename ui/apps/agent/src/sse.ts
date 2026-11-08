import { createParser } from "eventsource-parser";

/** Yields { event, data } for each event in a text/event-stream body (unnamed events are "message"). */
export async function* readEvents(
  body: ReadableStream<Uint8Array>,
): AsyncGenerator<{ event: string; data: string }> {
  const decoder = new TextDecoder();
  const queue: { event: string; data: string }[] = [];
  const parser = createParser({
    onEvent(msg) {
      queue.push({ event: msg.event || "message", data: msg.data });
    },
  });
  for await (const chunk of body) {
    parser.feed(decoder.decode(chunk, { stream: true }));
    for (const item of queue.splice(0)) yield item;
  }
  parser.feed(decoder.decode());
  parser.reset({ consume: true });
  for (const item of queue.splice(0)) yield item;
}
