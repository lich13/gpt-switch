//! Narrow, root-owned power service. No arbitrary command/path is accepted over XPC.
#import <Foundation/Foundation.h>
#import <Security/Security.h>
#import <xpc/xpc.h>
#include <sys/stat.h>
#include <unistd.h>
#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <sys/file.h>

static NSString *const Label = @"com.lich13.gpt-switch.power-helper";
static NSString *const Binary = @"/Library/PrivilegedHelperTools/com.lich13.gpt-switch.power-helper";
static NSString *const Plist = @"/Library/LaunchDaemons/com.lich13.gpt-switch.power-helper.plist";
static NSString *const Root = @"/Library/Application Support/gpt-Switch Power";
static NSString *const Manifest = @"/Library/Application Support/gpt-Switch Power/client.json";
static NSString *const Restore = @"/Library/Application Support/gpt-Switch Power/restore.json";
static NSString *const InstallReady = @"/Library/Application Support/gpt-Switch Power/install-ready.json";
static NSString *const AppBinary = @"/Applications/lich13-switch.app/Contents/MacOS/lich13-switch";
static NSDictionary *Approved;
#ifdef POWER_TEST
static NSDictionary *TestState;
static BOOL TestFailSecond, TestFailWrite;
static NSMutableDictionary *TestFiles;
#endif
static NSDictionary *failure(NSString *code, NSString *message) {
    return @{@"error":@{@"code":code,@"message":message}};
}
static NSData *jsonData(id obj) { return [NSJSONSerialization dataWithJSONObject:obj options:0 error:nil]; }
static NSDictionary *readJSON(NSString *path) {
#ifdef POWER_TEST
    return TestFiles[path];
#else
    NSData *data = [NSData dataWithContentsOfFile:path];
    if (!data || data.length > 16384) return nil;
    id value = [NSJSONSerialization JSONObjectWithData:data options:0 error:nil];
    return [value isKindOfClass:NSDictionary.class] ? value : nil;
#endif
}
static BOOL safeRootMetadata(uid_t uid, mode_t mode, BOOL directory) {
    return uid==0 && !(mode&0022) && (directory ? S_ISDIR(mode) : S_ISREG(mode));
}
static BOOL safeRootPath(NSString *path, BOOL directory) {
    struct stat st;
    return lstat(path.fileSystemRepresentation,&st)==0 && safeRootMetadata(st.st_uid,st.st_mode,directory);
}
static BOOL atomicData(NSString *path, NSData *data, mode_t mode) {
    NSString *tmp = [path stringByAppendingFormat:@".%@", NSUUID.UUID.UUIDString];
    int fd = open(tmp.fileSystemRepresentation, O_WRONLY|O_CREAT|O_EXCL|O_NOFOLLOW, mode);
    if(fd<0) return NO;
    const char *bytes=data.bytes; size_t left=data.length;
    BOOL ok=YES;
    while(left){ssize_t n=write(fd,bytes,left); if(n<0&&errno==EINTR)continue; if(n<=0){ok=NO;break;} bytes+=n;left-=(size_t)n;}
    if(fchmod(fd,mode)||fsync(fd))ok=NO;
    close(fd);
    if(ok)ok=rename(tmp.fileSystemRepresentation,path.fileSystemRepresentation)==0;
    if(!ok)unlink(tmp.fileSystemRepresentation);
    return ok;
}
static BOOL writeJSON(NSString *path, NSDictionary *data) {
#ifdef POWER_TEST
    if(TestFailWrite)return NO;
    if(data)TestFiles[path]=data;else[TestFiles removeObjectForKey:path];
    return YES;
#else
    if(!data)return unlink(path.fileSystemRepresentation)==0||errno==ENOENT;
    return atomicData(path,jsonData(data),0600);
#endif
}
static BOOL run(NSString *binary, NSArray<NSString *> *args, NSString **output) {
    NSTask *task=[NSTask new];task.executableURL=[NSURL fileURLWithPath:binary];task.arguments=args;
    task.environment=@{@"PATH":@"/usr/bin:/bin:/usr/sbin:/sbin",@"LANG":@"C"};
    NSPipe *pipe=[NSPipe pipe]; task.standardOutput=pipe;task.standardError=NSFileHandle.fileHandleWithNullDevice;
    dispatch_semaphore_t done=dispatch_semaphore_create(0);
    task.terminationHandler=^(NSTask *finished){dispatch_semaphore_signal(done);};
    NSError *error=nil;
    if(![task launchAndReturnError:&error])return NO;
    if(dispatch_semaphore_wait(done,dispatch_time(DISPATCH_TIME_NOW,3*NSEC_PER_SEC))!=0){
        if(task.running)kill(task.processIdentifier,SIGKILL);
        dispatch_semaphore_wait(done,dispatch_time(DISPATCH_TIME_NOW,NSEC_PER_SEC));
        return NO;
    }
    NSData *data=[pipe.fileHandleForReading readDataToEndOfFile];
    if(output)*output=[[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding];
    return task.terminationStatus==0;
}
static NSDictionary *powerState(void) {
#ifdef POWER_TEST
    return TestState;
#else
    NSString *general=nil,*custom=nil;
    if(!run(@"/usr/bin/pmset",@[@"-g"],&general)||!run(@"/usr/bin/pmset",@[@"-g",@"custom"],&custom))return nil;
    NSNumber *enabled=nil,*sleep=nil;BOOL battery=NO,supported=NO;
    for(NSString *line in [general componentsSeparatedByString:@"\n"]){
        NSArray *w=[[line componentsSeparatedByCharactersInSet:NSCharacterSet.whitespaceCharacterSet] filteredArrayUsingPredicate:[NSPredicate predicateWithFormat:@"length > 0"]];
        if(w.count>=2&&[w[0] isEqual:@"SleepDisabled"]&&([w[1] isEqual:@"0"]||[w[1] isEqual:@"1"]))enabled=@([w[1] boolValue]);
    }
    for(NSString *line in [custom componentsSeparatedByString:@"\n"]){
        if([line isEqual:@"Battery Power:"]){battery=YES;supported=YES;continue;}
        if(line.length&&![[NSCharacterSet whitespaceCharacterSet] characterIsMember:[line characterAtIndex:0]])battery=NO;
        NSArray *w=[[line componentsSeparatedByCharactersInSet:NSCharacterSet.whitespaceCharacterSet] filteredArrayUsingPredicate:[NSPredicate predicateWithFormat:@"length > 0"]];
        if(battery&&w.count>=2&&[w[0] isEqual:@"sleep"]){NSScanner *scan=[NSScanner scannerWithString:w[1]];NSInteger n=-1;if([scan scanInteger:&n]&&scan.isAtEnd&&n>=0&&n<=1440)sleep=@(n);}
    }
    if(!supported)return @{@"supported":@NO,@"enabled":@NO,@"batterySleep":@0};
    if(!enabled||!sleep)return nil;
    return @{@"supported":@YES,@"enabled":enabled,@"batterySleep":sleep};
#endif
}
static BOOL number(id value, NSInteger max) {
    if(![value isKindOfClass:NSNumber.class])return NO;
    double d=[value doubleValue];return isfinite(d)&&d>=0&&d<=max&&floor(d)==d;
}
static BOOL same(NSDictionary *a, NSDictionary *b) {
    return a&&b&&[a[@"enabled"] isEqual:b[@"enabled"]]&&[a[@"batterySleep"] isEqual:b[@"batterySleep"]];
}
static BOOL apply(BOOL enabled, NSInteger sleep) {
#ifdef POWER_TEST
    if(TestFailSecond){TestFailSecond=NO;TestState=@{@"supported":@YES,@"enabled":TestState[@"enabled"],@"batterySleep":@(sleep)};return NO;}
    TestState=@{@"supported":@YES,@"enabled":@(enabled),@"batterySleep":@(sleep)};return YES;
#else
    return enabled ? run(@"/usr/bin/pmset",@[@"-b",@"sleep",[@(sleep) stringValue]],NULL)&&run(@"/usr/bin/pmset",@[@"-b",@"disablesleep",@"1"],NULL) :
        run(@"/usr/bin/pmset",@[@"-b",@"disablesleep",@"0"],NULL)&&run(@"/usr/bin/pmset",@[@"-b",@"sleep",[@(sleep) stringValue]],NULL);
#endif
}
static NSDictionary *handle(NSDictionary *request) {
    NSString *op=request[@"op"];
    NSDictionary *before=powerState();
    if(!before)return failure(@"POWER",@"无法读取系统电源状态");
    if([op isEqual:@"state"])return @{@"state":before,@"version":@1};
    if([op isEqual:@"verifyInstall"]){
        NSString *transaction=readJSON(Manifest)[@"transaction"];
        if(![transaction isKindOfClass:NSString.class]||![request[@"transaction"] isEqual:transaction])
            return failure(@"POWER_INSTALL_VERIFY",@"没有匹配的助手安装事务");
        if(!writeJSON(InstallReady,@{@"transaction":transaction}))
            return failure(@"POWER_INSTALL_VERIFY",@"无法确认助手连接");
        return @{@"state":before,@"version":@1};
    }
    if(![op isEqual:@"set"]&&![op isEqual:@"prepareRemove"])return failure(@"POWER_PROTOCOL",@"不支持的电源操作");
    NSDictionary *record=readJSON(Restore);
#ifndef POWER_TEST
    if([[NSFileManager defaultManager] fileExistsAtPath:Restore]&&!record)return failure(@"POWER",@"电源恢复记录无效");
#endif
    if(record&&(!number(record[@"minutes"],1440)||![record[@"version"] isEqual:@1]||![record[@"expected"] isKindOfClass:NSDictionary.class]||!number(record[@"expected"][@"enabled"],1)||!number(record[@"expected"][@"batterySleep"],1440)))return failure(@"POWER",@"电源恢复记录无效");
    if([op isEqual:@"prepareRemove"]){
        if(record){
            if(!same(before,record[@"expected"]))return failure(@"CONFLICT",@"电源状态已被外部修改，请先处理后再移除助手");
            BOOL restored=apply(NO,[record[@"minutes"] integerValue]);
            NSDictionary *actual=powerState();
            if(!restored||!same(actual,@{@"enabled":@NO,@"batterySleep":record[@"minutes"]})){
                apply([before[@"enabled"] boolValue],[before[@"batterySleep"] integerValue]);
                return failure(@"POWER",@"恢复电源设置失败，已尝试回滚");
            }
            if(!writeJSON(Restore,nil)){
                apply([before[@"enabled"] boolValue],[before[@"batterySleep"] integerValue]);
                return failure(@"POWER",@"无法清理恢复记录，已尝试回滚");
            }
            return @{@"state":actual,@"version":@1};
        }
        if([before[@"enabled"] boolValue])return failure(@"CONFLICT",@"请先关闭合盖不休眠再移除助手");
        return @{@"state":before,@"version":@1};
    }
    if(!number(request[@"enabled"],1)||!number(request[@"minutes"],1440)||!number(request[@"beforeEnabled"],1)||!number(request[@"beforeSleep"],1440))return failure(@"POWER_PROTOCOL",@"电源参数无效");
    if(![before[@"supported"] boolValue])return failure(@"UNSUPPORTED",@"此设备不支持电池合盖控制");
    if(!same(before,@{@"enabled":request[@"beforeEnabled"],@"batterySleep":request[@"beforeSleep"]}))return failure(@"CONFLICT",@"电源状态已变化，请重新操作");
    BOOL enabled=[request[@"enabled"] boolValue];
    NSInteger minutes=[request[@"minutes"] integerValue];
    if(enabled&&minutes!=0)return failure(@"POWER_PROTOCOL",@"电源参数无效");
    if(record&&!same(before,record[@"expected"]))return failure(@"CONFLICT",@"电源状态已被外部修改，恢复记录已保留");
    if(!enabled&&record)minutes=[record[@"minutes"] integerValue];
    NSDictionary *desired=@{@"enabled":@(enabled),@"batterySleep":@(minutes)};
    if(same(before,desired))return @{@"state":before,@"version":@1};
    if(enabled&&!writeJSON(Restore,@{@"version":@1,@"minutes":before[@"batterySleep"],@"expected":desired}))return failure(@"POWER",@"无法保存电源恢复记录");
    BOOL ok=apply(enabled,minutes);NSDictionary *actual=powerState();
    if(!ok||!same(actual,desired)){
        BOOL restored=apply([before[@"enabled"] boolValue],[before[@"batterySleep"] integerValue]);
        if(restored&&same(powerState(),before))writeJSON(Restore,record);
        return failure(@"POWER",@"电源设置失败，已尝试恢复原状态");
    }
    if(!enabled&&!writeJSON(Restore,nil)){
        apply([before[@"enabled"] boolValue],[before[@"batterySleep"] integerValue]);
        return failure(@"POWER",@"恢复记录清理失败，已尝试回滚");
    }
    return @{@"state":actual,@"version":@1};
}
static NSString *codePath(SecCodeRef code) {
    CFDictionaryRef info=NULL;if(SecCodeCopySigningInformation(code,kSecCSSigningInformation,&info)!=errSecSuccess)return nil;
    NSURL *url=((__bridge NSDictionary *)info)[(__bridge NSString *)kSecCodeInfoMainExecutable];
    NSString *path=url.path;CFRelease(info);return path;
}
static BOOL identityMatches(uid_t uid, NSString *path, BOOL signatureValid, NSDictionary *approved) {
    return signatureValid && uid==[approved[@"uid"] unsignedIntValue] && [path isEqual:approved[@"path"]];
}
static BOOL authorized(xpc_connection_t peer, xpc_object_t message) {
    if(xpc_connection_get_euid(peer)!=[Approved[@"uid"] unsignedIntValue])return NO;
    SecCodeRef code=NULL;SecRequirementRef req=NULL;
    if(SecCodeCreateWithXPCMessage(message,kSecCSDefaultFlags,&code)!=errSecSuccess)return NO;
    OSStatus status=SecRequirementCreateWithString((__bridge CFStringRef)Approved[@"requirement"],kSecCSDefaultFlags,&req);
    BOOL ok=identityMatches(xpc_connection_get_euid(peer),codePath(code),status==errSecSuccess&&SecCodeCheckValidity(code,kSecCSStrictValidate,req)==errSecSuccess,Approved);
    if(req)CFRelease(req);CFRelease(code);return ok;
}
static BOOL safeDirectory(NSString *path) {
    if(mkdir(path.fileSystemRepresentation,0700)!=0&&errno!=EEXIST)return NO;
    return safeRootPath(path,YES);
}
static BOOL restoreFile(NSString *path, NSData *data, mode_t mode) {
    return data ? atomicData(path,data,mode) : unlink(path.fileSystemRepresentation)==0||errno==ENOENT;
}
#include "power_install.h"
#ifndef POWER_TEST
int main(int argc, const char *argv[]) {
    @autoreleasepool {
        if(argc==6&&strcmp(argv[1],"--install")==0){
            char *end=NULL;unsigned long uid=strtoul(argv[2],&end,10);
            NSDictionary *result=end&&!*end&&uid<=UINT32_MAX ? install((uid_t)uid,@(argv[3]),@(argv[4]),@(argv[5])) : installFailure(@"SIGNATURE",@"安装参数无效",@"unchanged");
            puts([[NSString alloc] initWithData:jsonData(result) encoding:NSUTF8StringEncoding].UTF8String);
            return 0;
        }
        if(argc!=1||geteuid()!=0||!safeRootPath(Root,YES)||!safeRootPath(Manifest,NO))return 1;
        Approved=readJSON(Manifest);
        if(!number(Approved[@"uid"],UINT32_MAX)||![Approved[@"path"] isEqual:AppBinary]||![Approved[@"requirement"] isKindOfClass:NSString.class])return 1;
        dispatch_queue_t queue=dispatch_queue_create("com.lich13.gpt-switch.power",DISPATCH_QUEUE_SERIAL);
        xpc_connection_t service=xpc_connection_create_mach_service(Label.UTF8String,queue,XPC_CONNECTION_MACH_SERVICE_LISTENER);
        xpc_connection_set_event_handler(service,^(xpc_object_t event){
            if(xpc_get_type(event)!=XPC_TYPE_CONNECTION)return;
            xpc_connection_t peer=(xpc_connection_t)event;
            if(xpc_connection_set_peer_code_signing_requirement(peer,[Approved[@"requirement"] UTF8String])!=0){xpc_connection_cancel(peer);return;}
            xpc_connection_set_target_queue(peer,queue);
            xpc_connection_set_event_handler(peer,^(xpc_object_t message){
                if(xpc_get_type(message)!=XPC_TYPE_DICTIONARY)return;
                xpc_object_t reply=xpc_dictionary_create_reply(message);if(!reply)return;
                NSDictionary *result=nil;BOOL remove=NO;
                if(!authorized(peer,message))result=failure(@"POWER_AUTH",@"应用版本未授权，请修复电源助手");
                else {
                    const char *raw=xpc_dictionary_get_string(message,"json");
                    if(raw&&strnlen(raw,4097)<=4096){
                        id request=[NSJSONSerialization JSONObjectWithData:[@(raw) dataUsingEncoding:NSUTF8StringEncoding] options:0 error:nil];
                        if([request isKindOfClass:NSDictionary.class]){
                            remove=[request[@"op"] isEqual:@"remove"];
                            result=handle(remove?@{@"op":@"prepareRemove"}:request);
                        }
                    }
                    if(!result)result=failure(@"POWER_PROTOCOL",@"电源请求无效");
                }
                if(remove&&!result[@"error"]){
                    // Remove only this service's fixed paths; no caller-supplied path.
                    NSData *manifest=[NSData dataWithContentsOfFile:Manifest],*plist=[NSData dataWithContentsOfFile:Plist],*binary=[NSData dataWithContentsOfFile:Binary];
                    if(!manifest||!plist||!binary)result=failure(@"POWER",@"无法读取电源助手注册文件");
                    else {
                        BOOL ok=unlink(Plist.fileSystemRepresentation)==0&&unlink(Binary.fileSystemRepresentation)==0&&unlink(Manifest.fileSystemRepresentation)==0;
                        if(!ok){restoreFile(Manifest,manifest,0600);restoreFile(Plist,plist,0644);restoreFile(Binary,binary,0755);result=failure(@"POWER",@"电源助手移除失败，原注册文件已尝试恢复");}
                        else {
                            unlink([[Root stringByAppendingPathComponent:@"install.lock"] fileSystemRepresentation]);
                            unlink(InstallReady.fileSystemRepresentation);
                            rmdir(Root.fileSystemRepresentation);
                            dispatch_after(dispatch_time(DISPATCH_TIME_NOW,NSEC_PER_SEC),queue,^{run(@"/bin/launchctl",@[@"bootout",[@"system/" stringByAppendingString:Label]],NULL);});
                        }
                    }
                }
                xpc_dictionary_set_string(reply,"json",[[[NSString alloc] initWithData:jsonData(result) encoding:NSUTF8StringEncoding] UTF8String]);
                xpc_connection_send_message(peer,reply);
            });
            xpc_connection_resume(peer);
        });
        xpc_connection_resume(service);dispatch_main();
    }
}
#else
#define CHECK(v) do { if(!(v)){fprintf(stderr,"power test failed at %d\n",__LINE__);return 1;} } while(0)
int main(void) {
    @autoreleasepool {
        CHECK(testInstallTransactions());
        NSDictionary *approved=@{@"uid":@501,@"path":AppBinary};
        CHECK(identityMatches(501,AppBinary,YES,approved));
        CHECK(!identityMatches(502,AppBinary,YES,approved));
        CHECK(!identityMatches(501,@"/tmp/lich13-switch",YES,approved));
        CHECK(!identityMatches(501,AppBinary,NO,approved));
        TestFiles=[NSMutableDictionary dictionary];TestState=@{@"supported":@YES,@"enabled":@NO,@"batterySleep":@0};
        TestFiles[Manifest]=@{@"transaction":@"fresh-install"};
        CHECK(handle(@{@"op":@"verifyInstall",@"transaction":@"old-install"})[@"error"]);
        CHECK(!readJSON(InstallReady));
        CHECK(!handle(@{@"op":@"verifyInstall",@"transaction":@"fresh-install"})[@"error"]);
        CHECK([readJSON(InstallReady)[@"transaction"] isEqual:@"fresh-install"]);
        CHECK([TestState[@"enabled"] isEqual:@NO]);
        [TestFiles removeObjectForKey:Manifest];[TestFiles removeObjectForKey:InstallReady];
        CHECK(handle(@{@"op":@"verifyInstall",@"transaction":@"fresh-install"})[@"error"]);
        CHECK(!handle(@{@"op":@"set",@"enabled":@YES,@"minutes":@0,@"beforeEnabled":@NO,@"beforeSleep":@0})[@"error"]);
        CHECK([readJSON(Restore)[@"minutes"] isEqual:@0]);
        CHECK(!handle(@{@"op":@"set",@"enabled":@NO,@"minutes":@1,@"beforeEnabled":@YES,@"beforeSleep":@0})[@"error"]);
        CHECK([TestState[@"batterySleep"] isEqual:@0]);CHECK(!readJSON(Restore));
        TestFailSecond=YES;TestState=@{@"supported":@YES,@"enabled":@NO,@"batterySleep":@7};
        CHECK(handle(@{@"op":@"set",@"enabled":@YES,@"minutes":@0,@"beforeEnabled":@NO,@"beforeSleep":@7})[@"error"]);
        CHECK([TestState[@"batterySleep"] isEqual:@7]);CHECK(!readJSON(Restore));
        TestFailWrite=YES;CHECK(handle(@{@"op":@"set",@"enabled":@YES,@"minutes":@0,@"beforeEnabled":@NO,@"beforeSleep":@7})[@"error"]);TestFailWrite=NO;
        CHECK([TestState[@"enabled"] isEqual:@NO]);
        CHECK(handle(@{@"op":@"set",@"enabled":@YES,@"minutes":@0,@"beforeEnabled":@NO,@"beforeSleep":@1})[@"error"]);
        CHECK(handle(@{@"op":@"run",@"command":@"true"})[@"error"]);
        CHECK(handle(@{@"op":@"set",@"enabled":@2,@"minutes":@0,@"beforeEnabled":@NO,@"beforeSleep":@7})[@"error"]);
        CHECK(!handle(@{@"op":@"set",@"enabled":@YES,@"minutes":@0,@"beforeEnabled":@NO,@"beforeSleep":@7})[@"error"]);
        TestState=@{@"supported":@YES,@"enabled":@YES,@"batterySleep":@3};
        CHECK(handle(@{@"op":@"prepareRemove"})[@"error"]);CHECK(readJSON(Restore));
        TestState=@{@"supported":@YES,@"enabled":@YES,@"batterySleep":@0};
        TestFailWrite=YES;
        CHECK(handle(@{@"op":@"prepareRemove"})[@"error"]);CHECK([TestState[@"enabled"] isEqual:@YES]);CHECK(readJSON(Restore));
        CHECK(handle(@{@"op":@"set",@"enabled":@NO,@"minutes":@7,@"beforeEnabled":@YES,@"beforeSleep":@0})[@"error"]);CHECK([TestState[@"enabled"] isEqual:@YES]);
        TestFailWrite=NO;
        CHECK(!handle(@{@"op":@"prepareRemove"})[@"error"]);CHECK([TestState[@"batterySleep"] isEqual:@7]);
        puts("power helper transaction tests passed (simulated)");
    }return 0;
}
#endif
