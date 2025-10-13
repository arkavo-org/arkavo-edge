cmake_minimum_required(VERSION 3.19)
project(arkavo-cef-bridge)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

set(CEF_ROOT "${CMAKE_CURRENT_SOURCE_DIR}/../../../vendor/cef" CACHE PATH "Path to CEF distribution")

if(NOT EXISTS "${CEF_ROOT}")
    message(FATAL_ERROR "CEF_ROOT not found at ${CEF_ROOT}. Please download CEF distribution.")
endif()

set(CMAKE_MODULE_PATH ${CMAKE_MODULE_PATH} "${CEF_ROOT}/cmake")

find_package(CEF REQUIRED)

add_definitions(-DDISABLE_JAVASCRIPT)
add_definitions(-DUSE_SANDBOX=0)

set(ARKAVO_CEF_SOURCES
    cef_app.cc
    cef_app.h
    dom_executor.cc
    dom_executor.h
    uds_client.cc
    uds_client.h
)

add_executable(arkavo-cef-renderer
    main.cc
    ${ARKAVO_CEF_SOURCES}
)

SET_EXECUTABLE_TARGET_PROPERTIES(arkavo-cef-renderer)

target_link_libraries(arkavo-cef-renderer
    libcef_lib
    libcef_dll_wrapper
    ${CEF_STANDARD_LIBS}
)

COPY_FILES("arkavo-cef-renderer" "${CEF_BINARY_FILES}" "${CEF_BINARY_DIR}" "${CMAKE_CURRENT_BINARY_DIR}")
COPY_FILES("arkavo-cef-renderer" "${CEF_RESOURCE_FILES}" "${CEF_RESOURCE_DIR}" "${CMAKE_CURRENT_BINARY_DIR}")

if(APPLE)
    set_target_properties(arkavo-cef-renderer PROPERTIES
        MACOSX_BUNDLE TRUE
        MACOSX_BUNDLE_INFO_PLIST "${CMAKE_CURRENT_SOURCE_DIR}/Info.plist"
    )
endif()
