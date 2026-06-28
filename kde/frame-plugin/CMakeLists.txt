cmake_minimum_required(VERSION 3.21)
project(better-wallpaper-plasma-plugin LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_AUTOMOC ON)
find_package(Qt6 6.6 REQUIRED COMPONENTS Core Network Qml Quick OpenGL Multimedia)
find_package(Qt6QuickPrivate 6.6 QUIET)

set(CARGO_TARGET_DIR "${CMAKE_SOURCE_DIR}/../../target")
if(CMAKE_BUILD_TYPE STREQUAL "Debug")
    set(GPU_RENDERER_LIB "${CARGO_TARGET_DIR}/debug/libgpu_renderer.so")
else()
    set(GPU_RENDERER_LIB "${CARGO_TARGET_DIR}/release/libgpu_renderer.so")
endif()

add_library(betterwallpaperplugin SHARED frameitem.cpp plugin.cpp)
target_link_libraries(betterwallpaperplugin PRIVATE
    Qt6::Core Qt6::Network Qt6::Qml Qt6::Quick Qt6::OpenGL Qt6::Multimedia
    "${GPU_RENDERER_LIB}")
target_include_directories(betterwallpaperplugin PRIVATE "${CMAKE_CURRENT_SOURCE_DIR}")
if(TARGET Qt6::QuickPrivate)
    target_link_libraries(betterwallpaperplugin PRIVATE Qt6::QuickPrivate)
    target_compile_definitions(betterwallpaperplugin PRIVATE HAS_QT_QUICK_PRIVATE)
endif()
set_target_properties(betterwallpaperplugin PROPERTIES
    PREFIX "lib"
    BUILD_RPATH "${CARGO_TARGET_DIR}/debug;${CARGO_TARGET_DIR}/release"
)
