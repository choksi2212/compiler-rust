/* P14 demo: actual record initialization, sorting and by-value returns.
   Explicit prototypes are used until the owned preprocessor/headers phase. */
int printf(const char*, ...);
struct Student {
    int roll;
    char name[16];
    int score;
};
_Static_assert(sizeof(int) == 4, "This demo uses the frozen LLP64 target");
struct Student best_of(struct Student a, struct Student b) {
    return a.score >= b.score ? a : b;
}
void sort_students(struct Student *students, int count) {
    for (int i = 1; i < count; i++) {
        struct Student current = students[i];
        int j = i - 1;
        while (j >= 0 && students[j].score < current.score) {
            students[j + 1] = students[j];
            j--;
        }
        students[j + 1] = current;
    }
}
int main(void) {
    _Alignas(16) struct Student students[] = {
        {.roll = 101, .name = "Asha", .score = 78},
        {.roll = 102, .name = "Nisha", .score = 92},
        {.roll = 103, .name = "Arjun", .score = 85}
    };
    int count = (int)(sizeof(students) / sizeof(students[0]));
    sort_students(students, count);
    printf("Sorted student records\n");
    for (int i = 0; i < count; i++) {
        printf("Rank %d: %s, score %d\n", i + 1, students[i].name, students[i].score);
    }
    struct Student winner = best_of(students[0], students[1]);
    printf("Winner: %s (%d)\n", winner.name, winner.score);
    return winner.score == 92 ? 0 : 1;
}
