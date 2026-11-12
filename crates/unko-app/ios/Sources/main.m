// iOS entry point: winit's event loop must start from the app's main thread,
// so the Rust static library exports `start_winit_app` and this file calls it.
#import <UIKit/UIKit.h>

extern void start_winit_app(void);

int main(int argc, char *argv[]) {
    (void)argc;
    (void)argv;
    start_winit_app();
    return 0;
}
