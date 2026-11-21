const original = router.navigate; router.navigate = () => false; router.refresh = original;
