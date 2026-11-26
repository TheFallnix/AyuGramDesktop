add_library(lib_storage OBJECT)
add_library(desktop-app::lib_storage ALIAS lib_storage)
init_target(lib_storage)

get_filename_component(storage_loc ${CMAKE_CURRENT_LIST_DIR}/../lib_storage REALPATH)
get_filename_component(ayu_src_loc ${CMAKE_CURRENT_LIST_DIR}/../SourceFiles REALPATH)

target_precompile_headers(lib_storage PRIVATE ${storage_loc}/storage/storage_pch.h)
nice_target_sources(lib_storage ${storage_loc}
PRIVATE
    storage/storage_clear_legacy.cpp
    storage/storage_clear_legacy_posix.cpp
    storage/storage_clear_legacy_win.cpp
    storage/storage_clear_legacy.h
    storage/storage_databases.cpp
    storage/storage_databases.h
    storage/storage_encrypted_file.cpp
    storage/storage_encrypted_file.h
    storage/storage_encryption.cpp
    storage/storage_encryption.h
    storage/cache/storage_cache_database.h
    storage/cache/storage_cache_types.cpp
    storage/cache/storage_cache_types.h
    storage/storage_pch.h
)
target_sources(lib_storage
PRIVATE
    ${ayu_src_loc}/ayu/storage/rust_cache_database.cpp
)

target_include_directories(lib_storage
PUBLIC
    ${storage_loc}
PRIVATE
    ${ayu_src_loc}
)

target_link_libraries(lib_storage
PUBLIC
    desktop-app::lib_base
PRIVATE
    desktop-app::external_xxhash
)
