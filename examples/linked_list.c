/* P14 demo: real heap nodes, compound literals and pointer traversal. */
int printf(const char*, ...);
void *malloc(unsigned long long);
void free(void*);
struct Node { int value; struct Node *next; };
int push(struct Node **head, int value) {
    struct Node *node = malloc(sizeof(struct Node));
    if (!node) return 0;
    *node = (struct Node){.value = value, .next = *head};
    *head = node;
    return 1;
}
void print_list(struct Node *node) {
    while (node) {
        printf("%d -> ", node->value);
        node = node->next;
    }
    printf("NULL\n");
}
void destroy(struct Node *node) {
    while (node) {
        struct Node *next = node->next;
        free(node);
        node = next;
    }
}
int main(void) {
    struct Node *head = 0;
    if (!push(&head, 10) || !push(&head, 20) || !push(&head, 30)) {
        destroy(head);
        return 1;
    }
    print_list(head);
    int sum = 0;
    for (struct Node *node = head; node; node = node->next) sum += node->value;
    printf("Sum = %d\n", sum);
    struct Node *removed = head;
    head = head->next;
    free(removed);
    print_list(head);
    destroy(head);
    return sum == 60 ? 0 : 2;
}
