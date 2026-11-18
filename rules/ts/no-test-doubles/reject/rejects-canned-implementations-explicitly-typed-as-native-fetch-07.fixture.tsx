type Transport = typeof fetch; function run(fetch) { const transport: Transport = async () => new Response(); }
