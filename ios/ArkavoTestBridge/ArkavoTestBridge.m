//
//  ArkavoTestBridge.m
//  iOS Bridge for Arkavo Test Harness
//

#import "ArkavoTestBridge.h"
#import <objc/runtime.h>

@interface ArkavoTestBridge ()
@property (nonatomic, weak) XCTestCase *testCase;
@property (nonatomic, strong) XCUIApplication *app;
@property (nonatomic, strong) NSMutableDictionary *stateSnapshots;
@end

@implementation ArkavoTestBridge

- (instancetype)initWithTestCase:(XCTestCase *)testCase {
    self = [super init];
    if (self) {
        _testCase = testCase;
        _app = [[XCUIApplication alloc] init];
        _stateSnapshots = [NSMutableDictionary dictionary];
    }
    return self;
}

#pragma mark - Action Execution

- (NSString *)executeAction:(NSString *)action params:(NSString *)params {
    NSError *error = nil;
    NSDictionary *paramDict = [NSJSONSerialization JSONObjectWithData:[params dataUsingEncoding:NSUTF8StringEncoding]
                                                              options:0
                                                                error:&error];
    if (error) {
        return [self errorResponse:@"Invalid JSON params" error:error];
    }
    
    @try {
        if ([action isEqualToString:@"tap"]) {
            return [self performTap:paramDict];
        } else if ([action isEqualToString:@"type"]) {
            return [self performType:paramDict];
        } else if ([action isEqualToString:@"swipe"]) {
            return [self performSwipe:paramDict];
        } else if ([action isEqualToString:@"wait"]) {
            return [self performWait:paramDict];
        } else if ([action isEqualToString:@"assert"]) {
            return [self performAssert:paramDict];
        } else {
            return [self errorResponse:@"Unknown action" error:nil];
        }
    } @catch (NSException *exception) {
        return [self errorResponse:exception.reason error:nil];
    }
}

- (NSString *)performTap:(NSDictionary *)params {
    XCUIElement *element = [self findElement:params];
    if (!element.exists) {
        return [self errorResponse:@"Element not found" error:nil];
    }
    
    [element tap];
    return [self successResponse:@{@"action": @"tap", @"element": params[@"identifier"] ?: @"unknown"}];
}

- (NSString *)performType:(NSDictionary *)params {
    XCUIElement *element = [self findElement:params];
    NSString *text = params[@"text"];
    
    if (!element.exists) {
        return [self errorResponse:@"Element not found" error:nil];
    }
    
    [element tap];
    [element typeText:text];
    return [self successResponse:@{@"action": @"type", @"text": text}];
}

- (NSString *)performSwipe:(NSDictionary *)params {
    NSString *direction = params[@"direction"];
    XCUIElement *element = params[@"identifier"] ? [self findElement:params] : self.app;
    
    if ([direction isEqualToString:@"up"]) {
        [element swipeUp];
    } else if ([direction isEqualToString:@"down"]) {
        [element swipeDown];
    } else if ([direction isEqualToString:@"left"]) {
        [element swipeLeft];
    } else if ([direction isEqualToString:@"right"]) {
        [element swipeRight];
    }
    
    return [self successResponse:@{@"action": @"swipe", @"direction": direction}];
}

- (NSString *)performWait:(NSDictionary *)params {
    NSTimeInterval duration = [params[@"duration"] doubleValue] ?: 1.0;
    [NSThread sleepForTimeInterval:duration];
    return [self successResponse:@{@"action": @"wait", @"duration": @(duration)}];
}

- (NSString *)performAssert:(NSDictionary *)params {
    XCUIElement *element = [self findElement:params];
    NSString *condition = params[@"condition"];
    
    BOOL result = NO;
    if ([condition isEqualToString:@"exists"]) {
        result = element.exists;
    } else if ([condition isEqualToString:@"enabled"]) {
        result = element.enabled;
    } else if ([condition isEqualToString:@"selected"]) {
        result = element.selected;
    }
    
    return [self successResponse:@{@"action": @"assert", @"condition": condition, @"result": @(result)}];
}

#pragma mark - Element Finding

- (XCUIElement *)findElement:(NSDictionary *)params {
    NSString *identifier = params[@"identifier"];
    NSString *type = params[@"type"] ?: @"any";
    NSString *label = params[@"label"];
    
    XCUIElementQuery *query = self.app.descendants(matchingType:XCUIElementTypeAny);
    
    if (identifier) {
        query = [query matchingIdentifier:identifier];
    }
    
    if (label) {
        NSPredicate *predicate = [NSPredicate predicateWithFormat:@"label == %@", label];
        query = [query matchingPredicate:predicate];
    }
    
    return query.firstMatch;
}

#pragma mark - State Management

- (NSString *)getCurrentState {
    NSMutableDictionary *state = [NSMutableDictionary dictionary];
    
    // Get current view hierarchy
    state[@"viewHierarchy"] = [self captureViewHierarchy];
    
    // Get app state through private APIs (if available)
    if ([self.app respondsToSelector:@selector(state)]) {
        state[@"appState"] = @([self.app performSelector:@selector(state)]);
    }
    
    // Get current screen
    state[@"currentScreen"] = [self identifyCurrentScreen];
    
    // Get visible elements
    state[@"visibleElements"] = [self getVisibleElements];
    
    return [self jsonStringFromDictionary:state];
}

- (NSString *)mutateState:(NSString *)entity action:(NSString *)action data:(NSString *)data {
    // This would integrate with your app's internal APIs
    // For now, we'll simulate state changes through UI
    
    if ([entity isEqualToString:@"user"] && [action isEqualToString:@"login"]) {
        // Simulate login through UI or deep link
        [self.app terminate];
        [self.app launchWithArguments:@[@"--test-user-logged-in"]];
        return [self successResponse:@{@"entity": entity, @"action": action}];
    }
    
    return [self errorResponse:@"State mutation not implemented" error:nil];
}

- (NSDictionary *)captureViewHierarchy {
    NSMutableDictionary *hierarchy = [NSMutableDictionary dictionary];
    XCUIElement *rootElement = self.app;
    
    [self traverseElement:rootElement into:hierarchy];
    
    return hierarchy;
}

- (void)traverseElement:(XCUIElement *)element into:(NSMutableDictionary *)dict {
    dict[@"type"] = @(element.elementType);
    dict[@"identifier"] = element.identifier ?: @"";
    dict[@"label"] = element.label ?: @"";
    dict[@"enabled"] = @(element.enabled);
    dict[@"selected"] = @(element.selected);
    
    if (element.children.count > 0) {
        NSMutableArray *children = [NSMutableArray array];
        for (XCUIElement *child in element.children.allElementsBoundByIndex) {
            NSMutableDictionary *childDict = [NSMutableDictionary dictionary];
            [self traverseElement:child into:childDict];
            [children addObject:childDict];
        }
        dict[@"children"] = children;
    }
}

- (NSString *)identifyCurrentScreen {
    // Use heuristics to identify current screen
    if ([self.app.navigationBars[@"Login"] exists]) {
        return @"LoginScreen";
    } else if ([self.app.navigationBars[@"Home"] exists]) {
        return @"HomeScreen";
    }
    // Add more screen detection logic
    return @"UnknownScreen";
}

- (NSArray *)getVisibleElements {
    NSMutableArray *elements = [NSMutableArray array];
    
    // Get all interactive elements
    NSArray *elementTypes = @[
        @(XCUIElementTypeButton),
        @(XCUIElementTypeTextField),
        @(XCUIElementTypeTextView),
        @(XCUIElementTypeSwitch),
        @(XCUIElementTypeSlider)
    ];
    
    for (NSNumber *type in elementTypes) {
        XCUIElementQuery *query = [self.app descendantsMatchingType:type.integerValue];
        for (XCUIElement *element in query.allElementsBoundByIndex) {
            if (element.exists && element.isHittable) {
                [elements addObject:@{
                    @"type": [self elementTypeString:element.elementType],
                    @"identifier": element.identifier ?: @"",
                    @"label": element.label ?: @"",
                    @"value": element.value ?: @"",
                    @"enabled": @(element.enabled)
                }];
            }
        }
    }
    
    return elements;
}

#pragma mark - Snapshot Management

- (NSData *)createSnapshot {
    NSMutableDictionary *snapshot = [NSMutableDictionary dictionary];
    
    // Capture UI state
    snapshot[@"viewHierarchy"] = [self captureViewHierarchy];
    snapshot[@"currentScreen"] = [self identifyCurrentScreen];
    
    // Capture app data (would need app cooperation)
    snapshot[@"timestamp"] = @([[NSDate date] timeIntervalSince1970]);
    
    return [NSKeyedArchiver archivedDataWithRootObject:snapshot requiringSecureCoding:NO error:nil];
}

- (void)restoreSnapshot:(NSData *)snapshotData {
    NSDictionary *snapshot = [NSKeyedUnarchiver unarchivedObjectOfClass:[NSDictionary class] 
                                                               fromData:snapshotData 
                                                                  error:nil];
    
    // Restore app to previous state
    // This typically requires app cooperation or deep links
    NSString *screen = snapshot[@"currentScreen"];
    
    // Navigate to the saved screen
    [self navigateToScreen:screen];
}

- (void)navigateToScreen:(NSString *)screenName {
    // Implement navigation logic based on your app's structure
    [self.app terminate];
    [self.app launchWithArguments:@[@"--open-screen", screenName]];
}

#pragma mark - AI-Driven Exploration

- (NSArray<NSString *> *)discoverAvailableActions {
    NSMutableArray *actions = [NSMutableArray array];
    
    // Find all tappable elements
    XCUIElementQuery *buttons = self.app.buttons;
    for (XCUIElement *button in buttons.allElementsBoundByIndex) {
        if (button.exists && button.isHittable) {
            [actions addObject:[NSString stringWithFormat:@"tap:%@", button.identifier ?: button.label]];
        }
    }
    
    // Find text fields
    XCUIElementQuery *textFields = self.app.textFields;
    for (XCUIElement *field in textFields.allElementsBoundByIndex) {
        if (field.exists && field.isHittable) {
            [actions addObject:[NSString stringWithFormat:@"type:%@", field.identifier ?: field.label]];
        }
    }
    
    // Add swipe actions
    [actions addObjectsFromArray:@[@"swipe:up", @"swipe:down", @"swipe:left", @"swipe:right"]];
    
    return actions;
}

- (NSDictionary *)analyzeCurrentScreen {
    return @{
        @"screen": [self identifyCurrentScreen],
        @"availableActions": [self discoverAvailableActions],
        @"visibleElements": [self getVisibleElements],
        @"elementCount": @(self.app.descendants(matchingType:XCUIElementTypeAny).count)
    };
}

#pragma mark - Helper Methods

- (NSString *)jsonStringFromDictionary:(NSDictionary *)dict {
    NSError *error;
    NSData *jsonData = [NSJSONSerialization dataWithJSONObject:dict options:0 error:&error];
    if (error) {
        return [self errorResponse:@"JSON serialization failed" error:error];
    }
    return [[NSString alloc] initWithData:jsonData encoding:NSUTF8StringEncoding];
}

- (NSString *)successResponse:(NSDictionary *)data {
    NSMutableDictionary *response = [NSMutableDictionary dictionaryWithDictionary:data];
    response[@"success"] = @YES;
    return [self jsonStringFromDictionary:response];
}

- (NSString *)errorResponse:(NSString *)message error:(NSError *)error {
    return [self jsonStringFromDictionary:@{
        @"success": @NO,
        @"error": message,
        @"details": error.localizedDescription ?: @""
    }];
}

- (NSString *)elementTypeString:(XCUIElementType)type {
    switch (type) {
        case XCUIElementTypeButton: return @"button";
        case XCUIElementTypeTextField: return @"textField";
        case XCUIElementTypeTextView: return @"textView";
        case XCUIElementTypeSwitch: return @"switch";
        case XCUIElementTypeSlider: return @"slider";
        default: return @"other";
    }
}

@end

#pragma mark - C Interface Implementation

void* arkavo_bridge_create(void* xctest_case) {
    XCTestCase *testCase = (__bridge XCTestCase *)xctest_case;
    ArkavoTestBridge *bridge = [[ArkavoTestBridge alloc] initWithTestCase:testCase];
    return (__bridge_retained void *)bridge;
}

void arkavo_bridge_destroy(void* bridge) {
    ArkavoTestBridge *testBridge = (__bridge_transfer ArkavoTestBridge *)bridge;
    testBridge = nil;
}

char* ios_bridge_execute_action(void* bridge, const char* action, const char* params) {
    ArkavoTestBridge *testBridge = (__bridge ArkavoTestBridge *)bridge;
    NSString *actionStr = [NSString stringWithUTF8String:action];
    NSString *paramsStr = [NSString stringWithUTF8String:params];
    NSString *result = [testBridge executeAction:actionStr params:paramsStr];
    return strdup([result UTF8String]);
}

char* ios_bridge_get_current_state(void* bridge) {
    ArkavoTestBridge *testBridge = (__bridge ArkavoTestBridge *)bridge;
    NSString *state = [testBridge getCurrentState];
    return strdup([state UTF8String]);
}

char* ios_bridge_mutate_state(void* bridge, const char* entity, const char* action, const char* data) {
    ArkavoTestBridge *testBridge = (__bridge ArkavoTestBridge *)bridge;
    NSString *entityStr = [NSString stringWithUTF8String:entity];
    NSString *actionStr = [NSString stringWithUTF8String:action];
    NSString *dataStr = [NSString stringWithUTF8String:data];
    NSString *result = [testBridge mutateState:entityStr action:actionStr data:dataStr];
    return strdup([result UTF8String]);
}

void* ios_bridge_create_snapshot(void* bridge, size_t* size) {
    ArkavoTestBridge *testBridge = (__bridge ArkavoTestBridge *)bridge;
    NSData *snapshot = [testBridge createSnapshot];
    *size = snapshot.length;
    void *buffer = malloc(snapshot.length);
    memcpy(buffer, snapshot.bytes, snapshot.length);
    return buffer;
}

void ios_bridge_restore_snapshot(void* bridge, const void* data, size_t size) {
    ArkavoTestBridge *testBridge = (__bridge ArkavoTestBridge *)bridge;
    NSData *snapshot = [NSData dataWithBytes:data length:size];
    [testBridge restoreSnapshot:snapshot];
}

void ios_bridge_free_string(char* s) {
    free(s);
}

void ios_bridge_free_data(void* data) {
    free(data);
}