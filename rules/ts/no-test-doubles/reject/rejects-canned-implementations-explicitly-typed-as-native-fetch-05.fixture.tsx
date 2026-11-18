type Original = typeof fetch; type Transport = Original; const transport: Transport = async () => new Response();
