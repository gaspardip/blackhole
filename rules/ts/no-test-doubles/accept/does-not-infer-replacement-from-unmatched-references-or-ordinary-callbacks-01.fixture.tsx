const original = router.navigate; router.navigate = () => false; other.navigate = original;
