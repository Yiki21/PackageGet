# Updater portable Linux package

Run `./updater` from the extracted directory. The host must provide a graphical
Wayland or X11 session, working graphics drivers, and the runtime libraries for
the libc variant named in the archive.

The portable tar archive and AppImage do not modify the host automatically.
Read-only workflows and user-scoped package managers work without system
integration. The tar archive includes a matching helper and policy; install
them explicitly when privileged writes through APT, DNF, Pacman, Zypper,
Portage, or XBPS are required:

```sh
cd system-integration
sudo ./install.sh
```

The installer writes only the fixed helper, Polkit policy, and scalable icon
under `/usr`. AppImages do not include this installer; use a native package, a
portable tar archive matching the host libc, or a source build for privileged
system-package operations.

The `glibc` archive targets general glibc-based distributions. The `musl`
archive targets Alpine Linux and compatible musl environments; it is not a
fully static desktop binary and is not intended to run on glibc distributions.
