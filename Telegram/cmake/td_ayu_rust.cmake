# This file is part of AyuGram Desktop.
#
# Integrates the Cargo Rust workspace (ayu-core, ayu-mtproto, ayu-storage, ayu-ffi)
# and cxx FFI headers into the CMake build.

add_library(td_ayu_rust INTERFACE)
add_library(tdesktop::td_ayu_rust ALIAS td_ayu_rust)

set(ayu_rust_gen_dir "${CMAKE_CURRENT_BINARY_DIR}/gen/ayu_rust")
set(ayu_rust_include_dir "${ayu_rust_gen_dir}/include")
set(ayu_rust_cxx_header "${ayu_rust_include_dir}/ayu-ffi/src/lib.rs.h")
set(ayu_rust_base_header "${ayu_rust_include_dir}/rust/cxx.h")
set(ayu_rust_target_dir "${CMAKE_CURRENT_BINARY_DIR}/rust-target")

file(MAKE_DIRECTORY "${ayu_rust_include_dir}")

find_package(Corrosion QUIET)
if (Corrosion_FOUND)
    corrosion_import_crate(
        MANIFEST_PATH "${CMAKE_SOURCE_DIR}/Cargo.toml"
        CRATES ayu-ffi
    )
    corrosion_set_env_vars(ayu_ffi
        "AYU_CXX_HEADER_OUT_DIR=${ayu_rust_include_dir}"
    )
    add_custom_target(td_ayu_rust_codegen
        DEPENDS ayu_ffi
    )
    target_link_libraries(td_ayu_rust
    INTERFACE
        ayu_ffi
    )
else()
    find_program(CARGO_EXECUTABLE cargo REQUIRED)

    if (WIN32 AND MSVC)
        set(ayu_rust_lib_name "ayu_ffi.lib")
    else()
        set(ayu_rust_lib_name "libayu_ffi.a")
    endif()

    set(ayu_rust_lib_debug "${ayu_rust_target_dir}/debug/${ayu_rust_lib_name}")
    set(ayu_rust_lib_release "${ayu_rust_target_dir}/release/${ayu_rust_lib_name}")

    file(GLOB_RECURSE ayu_rust_sources CONFIGURE_DEPENDS
        "${CMAKE_SOURCE_DIR}/Cargo.toml"
        "${CMAKE_SOURCE_DIR}/crates/*.rs"
        "${CMAKE_SOURCE_DIR}/crates/*/Cargo.toml"
    )

    add_custom_command(
        OUTPUT
            "${ayu_rust_cxx_header}"
            "${ayu_rust_base_header}"
        BYPRODUCTS
            "${ayu_rust_lib_debug}"
            "${ayu_rust_lib_release}"
        COMMAND
            "${CMAKE_COMMAND}" -E env
            --unset=MAKEFLAGS
            "AYU_CXX_HEADER_OUT_DIR=${ayu_rust_include_dir}"
            "${CARGO_EXECUTABLE}" build
            --manifest-path "${CMAKE_SOURCE_DIR}/Cargo.toml"
            --package ayu-ffi
            --target-dir "${ayu_rust_target_dir}"
            "$<IF:$<CONFIG:Debug>,--profile=dev,--release>"
        DEPENDS
            ${ayu_rust_sources}
        WORKING_DIRECTORY
            "${CMAKE_SOURCE_DIR}"
        COMMENT "Building Rust ayu-ffi backend and generating cxx headers"
        VERBATIM
    )

    add_custom_target(td_ayu_rust_codegen
        DEPENDS
            "${ayu_rust_cxx_header}"
            "${ayu_rust_base_header}"
    )

    add_library(td_ayu_rust_static STATIC IMPORTED GLOBAL)
    add_dependencies(td_ayu_rust_static td_ayu_rust_codegen)
    set_target_properties(td_ayu_rust_static PROPERTIES
        IMPORTED_LOCATION "${ayu_rust_lib_debug}"
        IMPORTED_LOCATION_DEBUG "${ayu_rust_lib_debug}"
        IMPORTED_LOCATION_RELEASE "${ayu_rust_lib_release}"
        IMPORTED_LOCATION_RELWITHDEBINFO "${ayu_rust_lib_release}"
        IMPORTED_LOCATION_MINSIZEREL "${ayu_rust_lib_release}"
    )

    target_link_libraries(td_ayu_rust
    INTERFACE
        td_ayu_rust_static
    )
endif()

target_include_directories(td_ayu_rust
INTERFACE
    "${ayu_rust_include_dir}"
    "${ayu_rust_target_dir}/cxxbridge"
)

if (WIN32)
    target_link_libraries(td_ayu_rust
    INTERFACE
        ws2_32
        userenv
        bcrypt
        ntdll
    )
elseif (APPLE)
    target_link_libraries(td_ayu_rust
    INTERFACE
        "-framework Security"
        "-framework CoreFoundation"
        resolv
    )
else()
    target_link_libraries(td_ayu_rust
    INTERFACE
        pthread
        dl
        m
    )
endif()
