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
    browser_client.cc
    browser_client.h
    # dom_executor.cc - Temporarily disabled due to CEF API compatibility (GetDOM removed in CEF 102+)
    # dom_executor.h
    uds_client.cc
    uds_client.h
)

if(APPLE)
    add_executable(arkavo-cef-renderer
        main.mm
        ${ARKAVO_CEF_SOURCES}
    )
else()
    add_executable(arkavo-cef-renderer
        main.cc
        ${ARKAVO_CEF_SOURCES}
    )
endif()

SET_EXECUTABLE_TARGET_PROPERTIES(arkavo-cef-renderer)

# Set full path to libcef_dll_wrapper
set(CEF_DLL_WRAPPER "${CEF_ROOT}/build_wrapper/libcef_dll_wrapper/libcef_dll_wrapper.a")

if(APPLE)
    # On macOS, link CEF framework binary directly (CEF uses flat framework structure)
    # Use symlink to avoid issues with spaces in path
    set(CEF_FRAMEWORK_BINARY "${CEF_ROOT}/Release/CEF.framework/libcef")

    if(EXISTS "${CEF_FRAMEWORK_BINARY}")
        message(STATUS "Found CEF framework binary: ${CEF_FRAMEWORK_BINARY}")

        # Link the framework binary directly
        target_link_libraries(arkavo-cef-renderer
            "${CEF_DLL_WRAPPER}"
            "${CEF_FRAMEWORK_BINARY}"
            ${CEF_STANDARD_LIBS}
        )

        # Set rpath and linker flags
        set_target_properties(arkavo-cef-renderer PROPERTIES
            BUILD_WITH_INSTALL_RPATH TRUE
            INSTALL_RPATH "@executable_path/../Frameworks"
            # Disable dead code stripping to preserve CEF symbols
            LINK_FLAGS_RELEASE ""
        )
        # Override CEF's default dead strip behavior
        string(REPLACE "-Wl,-dead_strip" "" CMAKE_EXE_LINKER_FLAGS "${CMAKE_EXE_LINKER_FLAGS}")

        # Set the install name for the CEF framework to use the real name
        add_custom_command(TARGET arkavo-cef-renderer POST_BUILD
            COMMAND install_name_tool -change
                "${CEF_FRAMEWORK_BINARY}"
                "@rpath/Chromium Embedded Framework.framework/Chromium Embedded Framework"
                "$<TARGET_FILE:arkavo-cef-renderer>"
            VERBATIM
        )
    else()
        message(FATAL_ERROR "CEF framework binary not found at ${CEF_FRAMEWORK_BINARY}")
    endif()
else()
    target_link_libraries(arkavo-cef-renderer
        libcef_lib
        "${CEF_DLL_WRAPPER}"
        ${CEF_STANDARD_LIBS}
    )
endif()

COPY_FILES("arkavo-cef-renderer" "${CEF_BINARY_FILES}" "${CEF_BINARY_DIR}" "${CMAKE_CURRENT_BINARY_DIR}")
COPY_FILES("arkavo-cef-renderer" "${CEF_RESOURCE_FILES}" "${CEF_RESOURCE_DIR}" "${CMAKE_CURRENT_BINARY_DIR}")

if(APPLE)
    set_target_properties(arkavo-cef-renderer PROPERTIES
        MACOSX_BUNDLE TRUE
        MACOSX_BUNDLE_INFO_PLIST "${CMAKE_CURRENT_SOURCE_DIR}/Info.plist"
    )

    # Copy CEF framework into the app bundle
    add_custom_command(TARGET arkavo-cef-renderer POST_BUILD
        COMMAND ${CMAKE_COMMAND} -E make_directory
            "$<TARGET_BUNDLE_CONTENT_DIR:arkavo-cef-renderer>/Frameworks"
        COMMAND ${CMAKE_COMMAND} -E copy_directory
            "${CEF_ROOT}/Release/Chromium Embedded Framework.framework"
            "$<TARGET_BUNDLE_CONTENT_DIR:arkavo-cef-renderer>/Frameworks/Chromium Embedded Framework.framework"
        VERBATIM
    )
endif()

# Install target for cmake crate
install(TARGETS arkavo-cef-renderer
    BUNDLE DESTINATION bin
    RUNTIME DESTINATION bin
)
