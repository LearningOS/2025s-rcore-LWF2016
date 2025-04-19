# 实现的功能
本节通过在TaskControlBlock中添加syscall_counter数组用来对任务的系统调用进行计数，并在TASK_MANAGER中增加了系统调用计数syscall_add和返回调用次数syscall_count的方法，计数方法在每次系统调用进行匹配前执行，保证sys_trace调用也被记为一次，但该调用可能会出现数组越界等问题。
# 简答1
# 简答2
## 问题1
在第三章的OS中刚进入`__restore`时`sp`即为内核栈的栈顶地址，它的第一种使用情景是OS即将运行用户程序时，提供用户程序的入口，第二种使用情景是用户程序使用系统调用结束后用来恢复调用前的任务上下文。
## 问题2
分别特殊处理了sstatus，sepc，sscratch。sstatus是恢复了Trap发生前CPU处在的特权级等信息，sepc是恢复了Trap发生前执行的最后一条指令的地址，sscratch是恢复了Trap发生前的内核栈地址。
# 简答3
跳过x2是因为它需要在后面特殊处理，跳过x4是因为应用程序并未使用到它
# 简答4
该指令之后sp指向内核栈地址，sscratch指向用户栈地址
# 简答5
`sret`
# 简答6
该指令之后sp指向用户栈地址，sscratch指向内核栈地址
# 简答7
`ecall`