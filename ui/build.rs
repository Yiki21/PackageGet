fn main() {
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=../assets/icons/updater.ico");
        let mut resource = winresource::WindowsResource::new();
        resource
            .set_icon("../assets/icons/updater.ico")
            .set("ProductName", "Updater")
            .set("FileDescription", "Updater package manager")
            .set("LegalCopyright", "Copyright (c) Yiki21")
            .set("OriginalFilename", "updater.exe");
        resource.compile().expect("compile Windows resources");
    }
}
