const original = router.navigate; router.navigate = async () => { throw new Error('failed'); }; try { await run(); } finally { router.navigate = original; }
