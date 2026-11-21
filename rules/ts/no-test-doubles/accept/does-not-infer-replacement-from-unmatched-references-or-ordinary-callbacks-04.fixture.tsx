const original = router.navigate; function configure(router) { router.navigate = () => false; } router.navigate = original;
