# Hostdisk sync admission

`run.py --output NEW_DIR` executes the actual device_sync body with modeled Exec
I/O and versioned devices. Eleven cases pass and four controls are rejected.
Hostdisk1.0 and earlier are refused before I/O because UPDATE acknowledged writes
without a real barrier. Version1.1 or newer still must return success from the
actual request. IOERR_NOCMD from either transport or device becomes explicit
ACTION_NOT_KNOWN; other failures remain errors. Other device families retain
their own UPDATE contract. This is a client admission test, not a real driver,
guest, storage durability or physical power-loss proof.
