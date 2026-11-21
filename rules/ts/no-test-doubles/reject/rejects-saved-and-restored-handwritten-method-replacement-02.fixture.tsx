const alias = router; const original = alias.navigate; router.navigate = () => undefined; alias.navigate = original;
