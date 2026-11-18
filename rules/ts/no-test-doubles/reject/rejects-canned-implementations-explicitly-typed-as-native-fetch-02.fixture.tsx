const transport: typeof globalThis.fetch = () => Promise.resolve(new Response("ok"));
