// Thin native bridge: no background capture stream, no retained frame cache.
#import <AppKit/AppKit.h>
#import <ScreenCaptureKit/ScreenCaptureKit.h>
#import <CoreText/CoreText.h>
#import <UniformTypeIdentifiers/UniformTypeIdentifiers.h>
#import <QuartzCore/QuartzCore.h>
#import <mach/mach.h>
#import <dispatch/dispatch.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

typedef struct { double x, y, width, height; } SPRect;
typedef struct { uint32_t id; SPRect bounds; double scale; } SPDisplay;
typedef struct { uint32_t width, height; char *error; int denied; } SPImage;
typedef struct { uint64_t resident, footprint, peakResident, peakFootprint; } SPMemory;

@interface SPScreenshot : NSObject {
@public CGImageRef image;
@public NSError *error;
}
@end
@implementation SPScreenshot
- (void)dealloc { if (image) CGImageRelease(image); }
@end

static NSRunningApplication *sp_previous_app;
void sp_begin_capture(void) {
    NSRunningApplication *front = NSWorkspace.sharedWorkspace.frontmostApplication;
    if (front.processIdentifier != getpid()) sp_previous_app = front;
}
void sp_end_capture(void) {
    NSRunningApplication *previous = sp_previous_app;
    sp_previous_app = nil;
    if (previous && !previous.terminated && NSApp.active) {
        [NSApp yieldActivationToApplication:previous];
        [previous activateFromApplication:NSRunningApplication.currentApplication options:0];
    }
}

static SPRect sp_rect(CGRect r) {
    return (SPRect){r.origin.x, r.origin.y, r.size.width, r.size.height};
}

int sp_display_for_cursor(SPDisplay *out) {
    @autoreleasepool {
        CGEventRef event = CGEventCreate(NULL);
        CGPoint point = event ? CGEventGetLocation(event) : CGPointZero;
        if (event) CFRelease(event);
        NSScreen *fallback = NSScreen.screens.firstObject;
        for (NSScreen *screen in NSScreen.screens) {
            CGDirectDisplayID display = [screen.deviceDescription[@"NSScreenNumber"] unsignedIntValue];
            CGRect bounds = CGDisplayBounds(display);
            if (CGRectContainsPoint(bounds, point)) {
                *out = (SPDisplay){display, sp_rect(bounds), screen.backingScaleFactor};
                return 1;
            }
        }
        if (!fallback) return 0;
        CGDirectDisplayID display = [fallback.deviceDescription[@"NSScreenNumber"] unsignedIntValue];
        *out = (SPDisplay){display, sp_rect(CGDisplayBounds(display)), fallback.backingScaleFactor};
        return 1;
    }
}

int sp_cursor(double *x, double *y) {
    CGEventRef event = CGEventCreate(NULL);
    if (!event) return 0;
    CGPoint p = CGEventGetLocation(event);
    CFRelease(event);
    *x = p.x; *y = p.y;
    return 1;
}

int sp_window_at(double x, double y, SPRect *out) {
    @autoreleasepool {
        CFArrayRef raw = CGWindowListCopyWindowInfo(kCGWindowListOptionOnScreenOnly |
                                                   kCGWindowListExcludeDesktopElements, kCGNullWindowID);
        if (!raw) return 0;
        NSArray *windows = CFBridgingRelease(raw);
        for (NSDictionary *window in windows) {
            if ([window[(__bridge NSString *)kCGWindowOwnerPID] intValue] == getpid() ||
                [window[(__bridge NSString *)kCGWindowLayer] intValue] != 0 ||
                [window[(__bridge NSString *)kCGWindowAlpha] doubleValue] <= 0) continue;
            CGRect rect;
            if (CGRectMakeWithDictionaryRepresentation((__bridge CFDictionaryRef)
                    window[(__bridge NSString *)kCGWindowBounds], &rect) &&
                !CGRectIsEmpty(rect) && CGRectContainsPoint(rect, CGPointMake(x, y))) {
                *out = sp_rect(rect);
                return 1;
            }
        }
        return 0;
    }
}

int sp_screen_permission(int request) {
    if (CGPreflightScreenCaptureAccess()) return 1;
    return request ? CGRequestScreenCaptureAccess() : 0;
}

void sp_open_screen_settings(void) {
    @autoreleasepool {
        [NSWorkspace.sharedWorkspace openURL:[NSURL URLWithString:
            @"x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"]];
    }
}

// Called from a Rust worker, never the AppKit main thread. Completion blocks own
// their state; a timeout cannot leave a callback referring to a returned stack.
SPImage sp_capture(uint32_t displayID, uint32_t width, uint32_t height, uint8_t *pixels) {
    @autoreleasepool {
        SPImage result = {0};
        if (!CGPreflightScreenCaptureAccess()) {
            result.denied = 1;
            return result;
        }
        dispatch_semaphore_t ready = dispatch_semaphore_create(0);
        __block SCShareableContent *content = nil;
        __block NSError *contentError = nil;
        [SCShareableContent getShareableContentExcludingDesktopWindows:NO onScreenWindowsOnly:YES
            completionHandler:^(SCShareableContent *value, NSError *error) {
                content = value; contentError = error;
                dispatch_semaphore_signal(ready);
            }];
        if (dispatch_semaphore_wait(ready, dispatch_time(DISPATCH_TIME_NOW, 10 * NSEC_PER_SEC))) {
            result.error = strdup("ScreenCaptureKit display enumeration timed out");
            return result;
        }
        SCDisplay *display = nil;
        for (SCDisplay *candidate in content.displays) {
            if (candidate.displayID == displayID) { display = candidate; break; }
        }
        if (!display) {
            result.denied = !CGPreflightScreenCaptureAccess();
            result.error = strdup(contentError.localizedDescription.UTF8String ?: "Display is unavailable");
            return result;
        }
        SCContentFilter *filter = [[SCContentFilter alloc] initWithDisplay:display excludingWindows:@[]];
        SCStreamConfiguration *config = [SCStreamConfiguration new];
        config.width = width;
        config.height = height;
        config.showsCursor = NO;
        config.scalesToFit = YES;
        // Default sourceRect captures the full display, including the menu bar.
        dispatch_semaphore_t captured = dispatch_semaphore_create(0);
        SPScreenshot *capture = [SPScreenshot new];
        [SCScreenshotManager captureImageWithFilter:filter configuration:config
            completionHandler:^(CGImageRef value, NSError *error) {
                // The block retains its result owner even after a timeout.
                // No pointer to the caller's pixels escapes into a callback.
                if (value) capture->image = CGImageRetain(value);
                capture->error = error;
                dispatch_semaphore_signal(captured);
            }];
        if (dispatch_semaphore_wait(captured, dispatch_time(DISPATCH_TIME_NOW, 10 * NSEC_PER_SEC))) {
            result.error = strdup("ScreenCaptureKit screenshot timed out");
            return result;
        }
        CGImageRef image = capture->image;
        if (!image) {
            result.denied = !CGPreflightScreenCaptureAccess();
            result.error = strdup(capture->error.localizedDescription.UTF8String ?: "Screenshot failed");
            return result;
        }
        if (CGImageGetWidth(image) != width || CGImageGetHeight(image) != height) {
            result.error = strdup("ScreenCaptureKit returned an unexpected pixel size");
            return result;
        }
        result.width = width; result.height = height;
        CGColorSpaceRef srgb = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
        CGContextRef context = pixels ? CGBitmapContextCreate(pixels, width, height, 8,
            (size_t)width * 4, srgb, kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Big) : NULL;
        if (srgb) CFRelease(srgb);
        if (!context) {
            result.error = strdup("Could not allocate screenshot pixels");
            return result;
        }
        CGContextSetBlendMode(context, kCGBlendModeCopy);
        CGContextSetInterpolationQuality(context, kCGInterpolationNone);
        CGContextDrawImage(context, CGRectMake(0, 0, width, height), image);
        CGContextRelease(context);
        for (size_t i = 3; i < (size_t)width * height * 4; i += 4) pixels[i] = 255;
        return result;
    }
}

void sp_free(void *pointer) { free(pointer); }

void sp_flush_compositor(void) { [CATransaction flush]; }

// NSView belongs to winit. AppKit calls below run on the main event-loop thread.
void sp_configure_window(void *viewPointer, int overlay, const SPDisplay *display) {
    NSView *view = (__bridge NSView *)viewPointer;
    NSWindow *window = view.window;
    window.animationBehavior = NSWindowAnimationBehaviorNone;
    window.hasShadow = NO;
    window.hidesOnDeactivate = NO;
    window.collectionBehavior = NSWindowCollectionBehaviorCanJoinAllSpaces |
        NSWindowCollectionBehaviorFullScreenAuxiliary | NSWindowCollectionBehaviorStationary;
    window.level = overlay ? CGWindowLevelForKey(kCGScreenSaverWindowLevelKey) : NSFloatingWindowLevel;
    if (overlay && display) {
        CGFloat mainHeight = NSScreen.screens.firstObject.frame.size.height;
        SPRect rect = display->bounds;
        [window setFrame:NSMakeRect(rect.x, mainHeight - rect.y - rect.height,
                                   rect.width, rect.height) display:NO];
    }
}

void sp_information(const char *title, const char *message) {
    @autoreleasepool {
        NSAlert *alert = [NSAlert new];
        alert.messageText = [NSString stringWithUTF8String:title];
        alert.informativeText = [NSString stringWithUTF8String:message];
        [alert addButtonWithTitle:@"好"];
        [NSApp activate];
        [alert runModal];
    }
}

char *sp_save_path(void) {
    @autoreleasepool {
        NSSavePanel *panel = NSSavePanel.savePanel;
        panel.title = @"保存截图";
        panel.nameFieldStringValue = @"截图.png";
        panel.canCreateDirectories = YES;
        panel.allowedContentTypes = @[[UTType typeWithIdentifier:@"public.png"],
                                     [UTType typeWithIdentifier:@"public.jpeg"]];
        [NSApp activate];
        if ([panel runModal] != NSModalResponseOK) return NULL;
        return strdup(panel.URL.fileSystemRepresentation);
    }
}

char *sp_chinese_font(void) {
    @autoreleasepool {
        CTFontRef font = CTFontCreateWithName(CFSTR("PingFangSC-Regular"), 14, NULL);
        CFURLRef url = font ? CTFontCopyAttribute(font, kCTFontURLAttribute) : NULL;
        char *path = url ? strdup([(__bridge NSURL *)url fileSystemRepresentation]) : NULL;
        if (url) CFRelease(url);
        if (font) CFRelease(font);
        return path;
    }
}

int sp_memory(SPMemory *out) {
    task_vm_info_data_t info = {0};
    mach_msg_type_number_t count = TASK_VM_INFO_COUNT;
    if (task_info(mach_task_self(), TASK_VM_INFO, (task_info_t)&info, &count) != KERN_SUCCESS) return 0;
    *out = (SPMemory){info.resident_size, info.phys_footprint, info.resident_size_peak,
                     info.ledger_phys_footprint_peak};
    return 1;
}
