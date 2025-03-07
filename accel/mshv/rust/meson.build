rust_mshv_ss = ss.source_set()
target_dir = meson.current_build_dir()
output_dir = target_dir + '/release'
cargo_config = meson.current_source_dir() + '/Cargo.toml'
lib_path = output_dir + '/libqemu_mshv.a'
hdr_path = output_dir + '/qemu-mshv.h'

build_mshv = custom_target('libmshv-build',
  command: ['cargo', 'build', '--release', '--target-dir', target_dir, '--manifest-path', cargo_config],
  output: 'release',  # This is the expected output from cargo
  depend_files: ['src/lib.rs', 'build.rs'],  # Dependencies for rebuild (e.g., source files)
)

libmshv_rs = custom_target('libmshv',
  command : ['cp', lib_path, target_dir + '/libqemu_mshv.a'],
  output : 'libqemu_mshv.a',
  depends : [build_mshv]
)

mshv_generated_h = custom_target('rust_to_c_header',
  command : ['cp', hdr_path, target_dir + '/qemu-mshv.h'],
  output : 'qemu-mshv.h',
  depends : [build_mshv]
)
genh += mshv_generated_h

#_libmshv_rs = static_library('mshv', libmshv_rs)
rust_mshv_ss.add(declare_dependency(
  link_whole: [libmshv_rs],
))

specific_ss.add_all(rust_mshv_ss)
