fn main() {
    // dlopen/dlsym: in libc itself on newer glibc, in libdl before 2.34.
    println!("cargo:rustc-link-lib=dl");
}
