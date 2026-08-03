# Updater portable Linux package

Run `./updater` from the extracted directory. The host must provide a graphical
Wayland or X11 session, working graphics drivers, and the runtime libraries for
the libc variant named in the archive.

The portable tar archive and AppImage do not install Updater's fixed-path
Polkit helper or policy. Read-only workflows and user-scoped package managers
work without that system integration. Privileged writes through APT, DNF,
Pacman, or Zypper require the helper and policy installed by an Updater DEB,
RPM, or Arch package.

The `glibc` archive targets general glibc-based distributions. The `musl`
archive targets Alpine Linux and compatible musl environments; it is not a
fully static desktop binary and is not intended to run on glibc distributions.
