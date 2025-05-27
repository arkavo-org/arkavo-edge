//
//  ArkavoTestBridge.h
//  iOS Bridge for Arkavo Test Harness
//

#import <Foundation/Foundation.h>
#import <XCTest/XCTest.h>

NS_ASSUME_NONNULL_BEGIN

@interface ArkavoTestBridge : NSObject

// Initialize bridge with test case
- (instancetype)initWithTestCase:(XCTestCase *)testCase;

// Execute UI actions
- (NSString *)executeAction:(NSString *)action params:(NSString *)params;

// State management
- (NSString *)getCurrentState;
- (NSString *)mutateState:(NSString *)entity action:(NSString *)action data:(NSString *)data;

// Snapshot management
- (NSData *)createSnapshot;
- (void)restoreSnapshot:(NSData *)snapshotData;

// AI-driven exploration
- (void)enableIntelligentExploration;
- (NSArray<NSString *> *)discoverAvailableActions;
- (NSDictionary *)analyzeCurrentScreen;

@end

// C interface for FFI
extern "C" {
    void* arkavo_bridge_create(void* xctest_case);
    void arkavo_bridge_destroy(void* bridge);
    
    char* ios_bridge_execute_action(void* bridge, const char* action, const char* params);
    char* ios_bridge_get_current_state(void* bridge);
    char* ios_bridge_mutate_state(void* bridge, const char* entity, const char* action, const char* data);
    
    void* ios_bridge_create_snapshot(void* bridge, size_t* size);
    void ios_bridge_restore_snapshot(void* bridge, const void* data, size_t size);
    
    void ios_bridge_free_string(char* s);
    void ios_bridge_free_data(void* data);
}

NS_ASSUME_NONNULL_END