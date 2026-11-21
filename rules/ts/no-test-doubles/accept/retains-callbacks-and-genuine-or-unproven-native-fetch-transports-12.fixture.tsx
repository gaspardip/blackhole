type Transport = typeof fetch; function run() { type Transport = () => Promise<Response>; const transport: Transport = async () => new Response(); }
