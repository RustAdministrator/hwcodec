#ifndef MFX_VPL_COMPAT_H
#define MFX_VPL_COMPAT_H

// Two functions of the Media SDK API that oneVPL no longer has.
//
// The code in this directory is written against the Media SDK headers. Their C++ wrapper,
// MFXVideoSession, has virtual methods that call MFXDoWork and
// MFXVideoCORE_SetBufferAllocator. Nothing here calls those methods, but being virtual they
// are in the vtable, so the linker wants both functions. The Media SDK dispatcher (libmfx)
// has them; the oneVPL dispatcher (vpl), which a current FFmpeg build links for Quick
// Sync, does not, and the link fails with two unresolved externals.
//
// The definitions below are used only when the dispatcher that is linked has none: the
// /alternatename directive names a fallback for a symbol that would otherwise be undefined.
// They report "unsupported", which is what oneVPL does for the removed part of the API.
//
// Include this file in exactly one source file that is always part of the link (it defines
// functions): a linker directive is only read from an object that is linked anyway.

#include <mfxdefs.h>
#include <mfxsession.h>
#include <mfxvideo.h>

extern "C" {

mfxStatus MFX_CDECL hwcodec_mfx_do_work_unsupported(mfxSession) {
  return MFX_ERR_UNSUPPORTED;
}

mfxStatus MFX_CDECL hwcodec_mfx_set_buffer_allocator_unsupported(mfxSession,
                                                                mfxBufferAllocator *) {
  return MFX_ERR_UNSUPPORTED;
}

} // extern "C"

#if defined(_M_IX86)
#pragma comment(linker, "/alternatename:_MFXDoWork=_hwcodec_mfx_do_work_unsupported")
#pragma comment(                                                                              \
    linker,                                                                                   \
    "/alternatename:_MFXVideoCORE_SetBufferAllocator=_hwcodec_mfx_set_buffer_allocator_unsupported")
#else
#pragma comment(linker, "/alternatename:MFXDoWork=hwcodec_mfx_do_work_unsupported")
#pragma comment(                                                                              \
    linker,                                                                                   \
    "/alternatename:MFXVideoCORE_SetBufferAllocator=hwcodec_mfx_set_buffer_allocator_unsupported")
#endif

#endif // MFX_VPL_COMPAT_H
