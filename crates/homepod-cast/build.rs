fn main() {
    if std::env::var("CARGO_CFG_WINDOWS").is_err() {
        return;
    }

    let mut resource = winresource::WindowsResource::new();
    resource
        .set_icon("../../assets/vulpicast.ico")
        .set("ProductName", "VulpiCast")
        .set(
            "FileDescription",
            "VulpiCast — local AirPlay 2 audio streaming",
        )
        .set("OriginalFilename", "VulpiCast.exe");
    resource
        .compile()
        .expect("compile VulpiCast Windows resources");
}
