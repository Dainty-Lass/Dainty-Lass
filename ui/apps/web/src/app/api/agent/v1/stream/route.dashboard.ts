import { asAgent } from "@/lib/fleet/agent-http";
import { attach } from "@/lib/fleet/registry";

export const dynamic = "force-dynamic";

/**
 * The agent's command channel: server-sent events, held open. A comment line every 20 s keeps proxies
 * from closing it as idle; the agent reconnects if it hears nothing for longer.
 */
export function GET(request: Request) {
  return asAgent(request, (agent) => {
    const encoder = new TextEncoder();
    let detach = () => {};
    let ping: ReturnType<typeof setInterval> | undefined;
    const stream = new ReadableStream<Uint8Array>({
      start(controller) {
        const write = (text: string) => {
          try {
            controller.enqueue(encoder.encode(text));
          } catch {
            detach();
          }
        };
        detach = attach(agent.id, {
          send: (event, data) => write(`event: ${event}\ndata: ${JSON.stringify(data)}\n\n`),
          close: () => {
            clearInterval(ping);
            try {
              controller.close();
            } catch {
              // Already closed by the client.
            }
          },
        });
        write(`event: hello\ndata: ${JSON.stringify({ agentId: agent.id })}\n\n`);
        ping = setInterval(() => write(": ping\n\n"), 20_000);
        request.signal.addEventListener("abort", () => {
          clearInterval(ping);
          detach();
        });
      },
      cancel() {
        clearInterval(ping);
        detach();
      },
    });
    return new Response(stream, {
      headers: {
        "Content-Type": "text/event-stream",
        "Cache-Control": "no-cache, no-transform",
        Connection: "keep-alive",
        // nginx buffers responses unless told not to, which would hold every command back.
        "X-Accel-Buffering": "no",
      },
    });
  });
}
